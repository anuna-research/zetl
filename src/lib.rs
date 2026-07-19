#[cfg(feature = "reason")]
pub mod acl;
pub mod assets;
pub mod cache;
pub mod cap;
pub mod cli;
pub mod crdt;
// The zetld daemon is Unix-only for now: its control plane is a Unix-domain
// socket and its lifecycle uses setsid()/kill(2). A Windows named-pipe
// implementation is a later slice; gating (rather than stubbing) keeps the
// x86_64-pc-windows-gnu cross-check honest about what actually runs there.
#[cfg(unix)]
pub mod daemon;
pub mod drift;
pub mod ecosystems;
pub mod extensions;
pub mod feed;
pub mod graph;
#[cfg(feature = "history")]
pub mod history;
pub mod hooks;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod merkle;
#[cfg(feature = "mobile")]
pub mod mobile_capture;
#[cfg(feature = "mobile")]
pub mod mobile_git;
#[cfg(feature = "mobile")]
pub mod mobile_state;
pub mod p2p;
pub mod parsers;
pub mod predicate_lints;
pub mod predicates;
pub mod rdf_export;
#[cfg(feature = "reason")]
pub mod reason;
pub mod scanner;
pub mod search;
pub mod search_index;
#[cfg(feature = "semantic")]
pub mod semantic;
pub mod simhash;
pub mod skill;
pub mod types;
pub mod user;
pub mod vcs;
pub mod view;
pub mod web;
