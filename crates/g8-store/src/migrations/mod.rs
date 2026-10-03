// Embed all migration files from this directory into the binary at compile time.
// Refinery discovers files matching V{n}__{name}.sql by convention.
refinery::embed_migrations!("src/migrations");

/// Checksum refinery records for V1__init.sql as it ships now: govern 0.1.0's
/// file, restored byte-for-byte. Editing V1 changes it and locks out stores.
pub const GOVERN_0_1_0_V1_CHECKSUM: u64 = 8761469828895239941;

/// Checksum g8 0.1.0 recorded for its in-place-edited V1__init.sql. Stores
/// carrying it are adopted on open; see `RusqliteStore::migrate`.
pub const G8_0_1_0_V1_CHECKSUM: u64 = 16160488255772028769;
