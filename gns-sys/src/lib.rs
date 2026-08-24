#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
// bindgen generates this file, so these lints do not apply to it.
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::useless_transmute)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_transmute_annotations)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
