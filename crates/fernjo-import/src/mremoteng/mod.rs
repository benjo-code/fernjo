//! mRemoteNG `confCons.xml` import (format 2.6).

pub mod crypto;
pub mod file;

pub use file::{Inherit, RawDocument, RawKind, RawNode, read};
