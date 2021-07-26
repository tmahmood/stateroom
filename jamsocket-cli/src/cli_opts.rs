use crate::room_id::RoomIdStrategy;
use clap::Clap;

#[derive(Clap)]
pub struct Opts {
    #[clap(subcommand)]
    pub subcommand: SubCommand,
}

#[derive(Clap)]
pub enum SubCommand {
    /// Run a dev server to host a given Jamsocket module.
    Serve(ServeCommand),
    /// Validate a given Jamsocket module.
    Validate(ValidateCommand),
}

#[derive(Clap)]
pub struct ServeCommand {
    /// The module (.wasm file) to serve.
    pub module: String,

    /// The port to serve on.
    #[clap(short, long, default_value = "8080")]
    pub port: u32,

    /// The strategy for assigning new room IDs.
    #[clap(short, long, default_value = "implicit")]
    pub rooms: RoomIdStrategy,
}

#[derive(Clap)]
pub struct ValidateCommand {
    /// The module (.wasm file) to validate.
    pub module: String,
}
