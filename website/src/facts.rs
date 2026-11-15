//! What the site says about the product, read from the product's own files by
//! `build.rs`: the version, the repository, the licence, the versions it is
//! tested with and the commands of the command line.

include!(concat!(env!("OUT_DIR"), "/facts.rs"));
