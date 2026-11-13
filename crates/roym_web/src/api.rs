//! Method reflection table for the web entrypoint service.

use syneroym_roym_core::method::{MethodInfo, NoParams};

use crate::app::{
    TARGETS, TRANSLATE, TargetsOutput, TranslateInput, TranslateOutput, WHOAMI, WhoamiOutput,
};

pub fn method_table() -> Vec<MethodInfo> {
    vec![
        MethodInfo::of::<NoParams, WhoamiOutput>(WHOAMI),
        MethodInfo::of::<NoParams, TargetsOutput>(TARGETS),
        MethodInfo::of::<TranslateInput, TranslateOutput>(TRANSLATE),
    ]
}
