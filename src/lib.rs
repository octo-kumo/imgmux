pub mod container;
pub mod format;
pub mod meta;
pub mod ops;
pub mod util;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn inv(msg: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    msg.into().into()
}
