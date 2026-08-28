// Embed all migration files from this directory into the binary at compile time.
// Refinery discovers files matching V{n}__{name}.sql by convention.
refinery::embed_migrations!("src/migrations");
