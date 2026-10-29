//! Authenticity's offline close-copying check, against the user's own library notes: nothing leaves the computer.
//!
//! - `text`: what a check leaves out of the text it is given (markdown, LaTeX, citations, quotations), its words with
//!   their places (normalised for case, punctuation, hyphenation, white space, accents and ligatures) and its sentences;
//! - `index`: the library's word-sequence (shingle) index, kept up to date note by note and stored between runs, and
//!   the matcher of verbatim and near-verbatim runs, with the field's fixed phrasing listed apart;
//! - `library`: how a library note is cut into passages and how a passage is named in the index, the same in the
//!   plugin (which reads notes through Obsidian's vault) and the native programs (which read them from disk).
//!
//! The check runs in two steps (`Index::plan`, then `Index::finish` with the passages it asks for), so the plugin can
//! read them asynchronously; the index is stored as bytes as well as in a file.

pub mod index;
pub mod library;
pub mod para;
pub mod text;

pub use index::{Common, Finding, Found, Index, Kind, NewSource, Options, ParaHit, ParaPlan, Plan, Query, Stats, K, PARA_CANDIDATES};
pub use text::Masks;
