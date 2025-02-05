use crate::WasmRuntimeError;
use anyhow::Result;
use byteorder::{LittleEndian, ReadBytesExt};
use stateroom::{ClientId, MessageRecipient, StateroomContext, StateroomService};
use std::{borrow::BorrowMut, sync::Arc};
use wasmtime::{Caller, Engine, Extern, Instance, Linker, Memory, Module, Store, TypedFunc, Val};
use wasmtime_wasi::sync::WasiCtxBuilder;
use wasmtime_wasi::WasiCtx;

const ENV: &str = "env";
const EXT_MEMORY: &str = "memory";
const EXT_FN_CONNECT: &str = "connect";
const EXT_FN_DISCONNECT: &str = "disconnect";
const EXT_FN_BINARY: &str = "binary";
const EXT_FN_MESSAGE: &str = "message";
const EXT_FN_SEND_MESSAGE: &str = "send_message";
const EXT_FN_SEND_BINARY: &str = "send_binary";
const EXT_FN_SET_TIMER: &str = "set_timer";
const EXT_FN_TIMER: &str = "timer";
const EXT_FN_INITIALIZE: &str = "initialize";
const EXT_FN_MALLOC: &str = "jam_malloc";
const EXT_FN_FREE: &str = "jam_free";
const EXT_JAMSOCKET_VERSION: &str = "JAMSOCKET_API_VERSION";
const EXT_JAMSOCKET_PROTOCOL: &str = "JAMSOCKET_API_PROTOCOL";

const EXPECTED_API_VERSION: i32 = 1;
const EXPECTED_PROTOCOL_VERSION: i32 = 0;

/// Hosts a [stateroom::StateroomService] implemented by a WebAssembly module.
pub struct WasmHost {
    store: Store<WasiCtx>,
    memory: Memory,

    fn_malloc: TypedFunc<u32, u32>,
    fn_free: TypedFunc<(u32, u32), ()>,
    fn_message: TypedFunc<(u32, u32, u32), ()>,
    fn_binary: TypedFunc<(u32, u32, u32), ()>,
    fn_connect: TypedFunc<u32, ()>,
    fn_disconnect: TypedFunc<u32, ()>,
    fn_timer: TypedFunc<(), ()>,
}

impl WasmHost {
    fn put_data(&mut self, data: &[u8]) -> Result<(u32, u32)> {
        #[allow(clippy::cast_possible_truncation)]
        let len = data.len() as u32;
        let pt = self.fn_malloc.call(&mut self.store, len)?;

        self.memory.write(&mut self.store, pt as usize, data)?;

        Ok((pt, len))
    }

    fn try_message(&mut self, client: ClientId, message: &str) -> Result<()> {
        let (pt, len) = self.put_data(message.as_bytes())?;

        self.fn_message
            .call(&mut self.store, (client.into(), pt, len))?;

        self.fn_free.call(&mut self.store, (pt, len))?;

        Ok(())
    }

    fn try_binary(&mut self, client: ClientId, message: &[u8]) -> Result<()> {
        let (pt, len) = self.put_data(message)?;

        self.fn_binary
            .call(&mut self.store, (client.into(), pt as u32, len))?;

        self.fn_free.call(&mut self.store, (pt, len))?;

        Ok(())
    }
}

impl StateroomService for WasmHost {
    fn message(&mut self, client: ClientId, message: &str) {
        if let Err(error) = self.try_message(client, message) {
            tracing::error!(?error, "Error calling `message` on wasm host");
        }
    }

    fn connect(&mut self, client: ClientId) {
        if let Err(error) = self.fn_connect.call(&mut self.store, client.into()) {
            tracing::error!(?error, "Error calling `connect` on wasm host");
        }
    }

    fn disconnect(&mut self, client: ClientId) {
        if let Err(error) = self.fn_disconnect.call(&mut self.store, client.into()) {
            tracing::error!(?error, "Error calling `disconnect` on wasm host");
        };
    }

    fn timer(&mut self) {
        if let Err(error) = self.fn_timer.call(&mut self.store, ()) {
            tracing::error!(?error, "Error calling `timer` on wasm host");
        };
    }

    fn binary(&mut self, client: ClientId, message: &[u8]) {
        if let Err(error) = self.try_binary(client, message) {
            tracing::error!(?error, "Error calling `binary` on wasm host");
        };
    }
}

#[inline]
fn get_memory<T>(caller: &mut Caller<'_, T>) -> Memory {
    match caller.get_export(EXT_MEMORY) {
        Some(Extern::Memory(mem)) => mem,
        _ => panic!(),
    }
}

#[inline]
fn get_string<'a, T>(
    caller: &'a Caller<'_, T>,
    memory: &'a Memory,
    start: u32,
    len: u32,
) -> Result<&'a str> {
    let data = get_u8_vec(caller, memory, start, len);
    std::str::from_utf8(data).map_err(|e| e.into())
}

#[inline]
fn get_u8_vec<'a, T>(
    caller: &'a Caller<'_, T>,
    memory: &'a Memory,
    start: u32,
    len: u32,
) -> &'a [u8] {
    let data = memory
        .data(caller)
        .get(start as usize..(start + len) as usize);
    match data {
        Some(data) => data,
        None => panic!(),
    }
}

pub fn get_global<T>(
    store: &mut Store<T>,
    memory: &mut Memory,
    instance: &Instance,
    name: &str,
) -> Result<i32> {
    #[allow(clippy::cast_sign_loss)]
    let i: u32 = {
        let mem_location = instance
            .get_global(store.borrow_mut(), name)
            .ok_or(WasmRuntimeError::CouldNotImportGlobal)?;

        match mem_location.get(store.borrow_mut()) {
            Val::I32(i) => Ok(i),
            _ => Err(WasmRuntimeError::CouldNotImportGlobal),
        }? as u32
    };

    #[allow(clippy::cast_possible_truncation)]
    let mut value = memory
        .data(store)
        .get(i as usize..(i as usize + std::mem::size_of::<i32>()))
        .ok_or(WasmRuntimeError::CouldNotImportGlobal)?;
    let result = value.read_i32::<LittleEndian>()?;
    Ok(result)
}

impl WasmHost {
    pub fn new(
        room_id: &str,
        module: &Module,
        engine: &Engine,
        context: &Arc<impl StateroomContext + Send + Sync + 'static>,
    ) -> Result<Self> {
        let wasi = WasiCtxBuilder::new().inherit_stdio().build();

        let mut store = Store::new(engine, wasi);
        let mut linker = Linker::new(engine);
        wasmtime_wasi::add_to_linker(&mut linker, |s| s)?;

        {
            #[allow(clippy::redundant_clone)]
            let context = context.clone();
            linker.func_wrap(
                ENV,
                EXT_FN_SEND_MESSAGE,
                move |mut caller: Caller<'_, WasiCtx>, client: i32, start: u32, len: u32| {
                    let memory = get_memory(&mut caller);
                    let message = get_string(&caller, &memory, start, len)?;

                    context.send_message(MessageRecipient::decode_i32(client), message);

                    Ok(())
                },
            )?;
        }

        {
            #[allow(clippy::redundant_clone)]
            let context = context.clone();
            linker.func_wrap(
                ENV,
                EXT_FN_SEND_BINARY,
                move |mut caller: Caller<'_, WasiCtx>, client: i32, start: u32, len: u32| {
                    let memory = get_memory(&mut caller);
                    let message = get_u8_vec(&caller, &memory, start, len);

                    context.send_binary(MessageRecipient::decode_i32(client), message);

                    Ok(())
                },
            )?;
        }

        {
            #[allow(clippy::redundant_clone)]
            let context = context.clone();
            linker.func_wrap(
                ENV,
                EXT_FN_SET_TIMER,
                move |_: Caller<'_, WasiCtx>, duration_ms: u32| {
                    context.set_timer(duration_ms);

                    Ok(())
                },
            )?;
        }

        let instance = linker.instantiate(&mut store, module)?;

        let initialize =
            instance.get_typed_func::<(u32, u32), (), _>(&mut store, EXT_FN_INITIALIZE)?;

        let fn_malloc = instance.get_typed_func::<u32, u32, _>(&mut store, EXT_FN_MALLOC)?;

        let fn_free = instance.get_typed_func::<(u32, u32), (), _>(&mut store, EXT_FN_FREE)?;

        let mut memory = instance
            .get_memory(&mut store, EXT_MEMORY)
            .ok_or(WasmRuntimeError::CouldNotImportMemory)?;

        {
            let room_id = room_id.as_bytes();
            #[allow(clippy::cast_possible_truncation)]
            let len = room_id.len() as u32;
            let pt = fn_malloc.call(&mut store, len)?;

            memory.write(&mut store, pt as usize, room_id)?;
            initialize.call(&mut store, (pt, len))?;

            fn_free.call(&mut store, (pt, len))?;
        }

        if get_global(&mut store, &mut memory, &instance, EXT_JAMSOCKET_VERSION)?
            != EXPECTED_API_VERSION
        {
            return Err(WasmRuntimeError::InvalidApiVersion.into());
        }

        if get_global(&mut store, &mut memory, &instance, EXT_JAMSOCKET_PROTOCOL)?
            != EXPECTED_PROTOCOL_VERSION
        {
            return Err(WasmRuntimeError::InvalidProtocolVersion.into());
        }

        let fn_connect = instance.get_typed_func::<u32, (), _>(&mut store, EXT_FN_CONNECT)?;

        let fn_disconnect = instance.get_typed_func::<u32, (), _>(&mut store, EXT_FN_DISCONNECT)?;

        let fn_timer = instance.get_typed_func::<(), (), _>(&mut store, EXT_FN_TIMER)?;

        let fn_message =
            instance.get_typed_func::<(u32, u32, u32), (), _>(&mut store, EXT_FN_MESSAGE)?;

        let fn_binary =
            instance.get_typed_func::<(u32, u32, u32), (), _>(&mut store, EXT_FN_BINARY)?;

        Ok(WasmHost {
            store,
            memory,
            fn_malloc,
            fn_free,
            fn_message,
            fn_binary,
            fn_connect,
            fn_disconnect,
            fn_timer,
        })
    }
}
