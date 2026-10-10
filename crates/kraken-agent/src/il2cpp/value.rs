use std::ffi::CString;
use std::ptr;

use super::api::{Ptr, api};
use super::meta::{Class, Field, Prim};

pub unsafe fn field_addr(object: Ptr, field: Field) -> *mut u8 {
    unsafe { object.cast::<u8>().add(field.offset()) }
}

fn field_readable(object: Ptr, field: Field) -> bool {
    !object.is_null() && super::guard::readable(unsafe { field_addr(object, field) }, size_of::<f64>())
}

pub unsafe fn get_ref(object: Ptr, field: Field) -> Ptr {
    if !field_readable(object, field) {
        return ptr::null_mut();
    }
    unsafe { ptr::read_unaligned(field_addr(object, field).cast::<Ptr>()) }
}

pub unsafe fn set_ref(object: Ptr, field: Field, value: Ptr) {
    unsafe { ptr::write_unaligned(field_addr(object, field).cast::<Ptr>(), value) };
}

pub unsafe fn get_f64(object: Ptr, field: Field) -> Option<f64> {
    if !field_readable(object, field) {
        return None;
    }
    let prim = field.prim()?;
    Some(unsafe { prim.read_f64(field_addr(object, field)) })
}

pub unsafe fn get_i64(object: Ptr, field: Field) -> Option<i64> {
    if !field_readable(object, field) {
        return None;
    }
    let prim = field.prim()?;
    Some(unsafe { prim.read_i64(field_addr(object, field)) })
}

pub unsafe fn get_bool(object: Ptr, field: Field) -> Option<bool> {
    unsafe { get_i64(object, field) }.map(|v| v != 0)
}

pub unsafe fn set_num(object: Ptr, field: Field, value: f64) -> bool {
    match field.prim() {
        Some(prim) => {
            unsafe { prim.write(field_addr(object, field), value) };
            true
        }
        None => false,
    }
}

pub unsafe fn inline_i64(base: *const u8, field: Field) -> Option<i64> {
    let prim = field.prim()?;
    Some(unsafe { prim.read_i64(base.add(field.inline_offset())) })
}

pub unsafe fn inline_f64(base: *const u8, field: Field) -> Option<f64> {
    let prim = field.prim()?;
    Some(unsafe { prim.read_f64(base.add(field.inline_offset())) })
}

pub unsafe fn inline_set(base: *mut u8, field: Field, value: f64) -> bool {
    match field.prim() {
        Some(prim) => {
            unsafe { prim.write(base.add(field.inline_offset()), value) };
            true
        }
        None => false,
    }
}

pub fn unbox(object: Ptr) -> *const u8 {
    unsafe { (api().il2cpp_object_unbox)(object) }.cast()
}

pub fn boxed_bool(object: Ptr) -> Option<bool> {
    (!object.is_null()).then(|| unsafe { ptr::read(unbox(object)) } != 0)
}

pub fn read_string(string: Ptr) -> String {
    if Class::of(string).is_none() {
        return String::new();
    }
    let len = unsafe { (api().il2cpp_string_length)(string) }.clamp(0, 1 << 20) as usize;
    let chars = unsafe { (api().il2cpp_string_chars)(string) };
    if !super::guard::readable(chars.cast(), len * 2) {
        return String::new();
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(chars, len) })
}

pub fn new_string(value: &str) -> Ptr {
    let text = CString::new(value.replace('\0', "")).unwrap_or_default();
    unsafe { (api().il2cpp_string_new)(text.as_ptr()) }
}

pub fn read_prim_pair(base: *const u8, class: Class) -> Option<(f64, f64)> {
    let fields: Vec<(Field, Prim)> = class
        .fields()
        .into_iter()
        .filter(|f| !f.is_static())
        .filter_map(|f| Some((f, f.prim()?)))
        .collect();
    let [(fx, px), (fy, py), ..] = fields.as_slice() else {
        return None;
    };
    unsafe {
        Some((
            px.read_f64(base.add(fx.inline_offset())),
            py.read_f64(base.add(fy.inline_offset())),
        ))
    }
}
