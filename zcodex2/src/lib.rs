//! Reader/writer for Zortrax `.zcodex2` files.
//!
//! Layout (little-endian), as observed in Z-Suite 3.7 output:
//! `u32 json_len | JSON metadata | u32 png_len | PNG preview | ZIP`.
//! The ZIP holds `AdditionalMetadata` (JSON), `ZCodeData` (binary command stream),
//! `ConfigurationData` (RSA/AES encrypted, opaque) and `UserSettingsData` (JSON).

mod container;
mod crc;
mod error;
mod models;
pub mod nrbf;
mod record;
mod store;

pub use container::{Zcodex2, ADDITIONAL_METADATA, CONFIGURATION_DATA, USER_SETTINGS_DATA, ZCODE_DATA};
pub use crc::crc8;
pub use error::{Error, Result};
pub use models::{decode_models, encode_models, Model};
pub use store::Store;
pub use record::{encode_records, parse_records, Axes, Command, Move, Rapid, Record};
