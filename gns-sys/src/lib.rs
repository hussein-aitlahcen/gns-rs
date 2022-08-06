#[allow(non_upper_case_globals)]
#[allow(non_snake_case)]
#[allow(non_camel_case_types)]
pub mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
