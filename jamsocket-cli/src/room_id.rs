use rand::distributions::Uniform;
use rand::{thread_rng, Rng};
use std::{
    fmt::{Debug, Display},
    str::FromStr,
};
use uuid::Uuid;

struct ShortRoomIdGenerator(pub usize);

impl RoomIdGenerator for ShortRoomIdGenerator {
    fn generate(&self) -> String {
        thread_rng()
            .sample_iter(&Uniform::from('A'..'Z'))
            .map(|c| c as char)
            .take(self.0)
            .collect()
    }
}

pub struct UuidRoomIdGenerator;

impl RoomIdGenerator for UuidRoomIdGenerator {
    fn generate(&self) -> String {
        let my_uuid = Uuid::new_v4();
        my_uuid.to_string()
    }
}

pub trait RoomIdGenerator {
    fn generate(&self) -> String;
}

pub enum RoomIdStrategy {
    Implicit,
    Explicit,
    Singleton,
    Generator(Box<dyn RoomIdGenerator>),
}

#[derive(Debug)]
pub struct BadGeneratorName(String);

impl Display for BadGeneratorName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Bad room ID generator '{}', expected one of {{singleton,short,uuid,api,implicit}}.",
            self.0
        )
    }
}

impl std::error::Error for BadGeneratorName {}

impl FromStr for RoomIdStrategy {
    type Err = BadGeneratorName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "api" => Ok(RoomIdStrategy::Explicit),
            "implicit" => Ok(RoomIdStrategy::Implicit),
            "short" => Ok(RoomIdStrategy::Generator(Box::new(ShortRoomIdGenerator(4)))),
            "uuid" => Ok(RoomIdStrategy::Generator(Box::new(UuidRoomIdGenerator))),
            "singleton" => Ok(RoomIdStrategy::Singleton),
            _ if s.starts_with("short") => {
                if let Some(num) = s.strip_prefix("short") {
                    let n: usize = num.parse().map_err(|_| BadGeneratorName(s.to_string()))?;
                    Ok(RoomIdStrategy::Generator(Box::new(ShortRoomIdGenerator(n))))
                } else {
                    panic!() // Should never get here.
                }
            }
            _ => Err(BadGeneratorName(s.to_string())),
        }
    }
}
