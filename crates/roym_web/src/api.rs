//! Method reflection table for the web entrypoint service.

use syneroym_roym_core::method::{MethodInfo, NoParams};

use crate::app::{WHOAMI, WhoamiOutput};

pub fn method_table() -> Vec<MethodInfo> {
    vec![MethodInfo::of::<NoParams, WhoamiOutput>(WHOAMI)]
}
