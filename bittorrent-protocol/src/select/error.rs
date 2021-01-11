//! Module for uber error types.

use crate::select::ut_metadata::error::{UtMetadataError, UtMetadataErrorKind};

error_chain! {
    types {
        UberError, UberErrorKind, UberResultExt;
    }

    links {
        Ut_Metadata(UtMetadataError, UtMetadataErrorKind);
    }
}
