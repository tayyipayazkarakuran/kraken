use std::ffi::{CStr, CString, c_char};
use std::ptr;

use super::api::{Ptr, api};

pub const HEADER: usize = 2 * size_of::<usize>();

const FIELD_STATIC: i32 = 0x10;
const FIELD_LITERAL: i32 = 0x40;

const CLASS_PROBE: usize = 16 * size_of::<usize>();

fn text(raw: *const c_char) -> &'static str {
    if !super::guard::readable(raw.cast(), 1) {
        return "";
    }
    unsafe { CStr::from_ptr(raw) }.to_str().unwrap_or("")
}

pub fn normalize(name: &str) -> String {
    let trimmed = name
        .strip_prefix('<')
        .and_then(|n| n.split_once('>'))
        .map_or(name, |(n, _)| n);
    trimmed.trim_start_matches('_').to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Class(pub Ptr);

unsafe impl Send for Class {}
unsafe impl Sync for Class {}

impl Class {
    pub fn from_ptr(ptr: Ptr) -> Option<Self> {
        (!ptr.is_null()).then_some(Self(ptr))
    }

    pub fn of(object: Ptr) -> Option<Self> {
        if !super::guard::pointer(object.cast()) {
            return None;
        }
        let class = unsafe { std::ptr::read(object.cast::<Ptr>()) };
        if !super::guard::pointer(class.cast()) || !super::guard::readable(class.cast(), CLASS_PROBE) {
            return None;
        }
        let name = unsafe { (api().il2cpp_class_get_name)(class) };
        super::guard::readable(name.cast(), 1).then_some(Self(class))
    }

    pub fn name(self) -> &'static str {
        text(unsafe { (api().il2cpp_class_get_name)(self.0) })
    }

    pub fn namespace(self) -> &'static str {
        text(unsafe { (api().il2cpp_class_get_namespace)(self.0) })
    }

    pub fn full_name(self) -> String {
        match self.namespace() {
            "" => self.name().to_owned(),
            ns => format!("{ns}.{}", self.name()),
        }
    }

    pub fn parent(self) -> Option<Self> {
        Self::from_ptr(unsafe { (api().il2cpp_class_get_parent)(self.0) })
    }

    pub fn lineage(self) -> impl Iterator<Item = Self> {
        std::iter::successors(Some(self), |c| c.parent())
    }

    pub fn ty(self) -> Type {
        Type(unsafe { (api().il2cpp_class_get_type)(self.0) })
    }

    pub fn type_object(self) -> Ptr {
        unsafe { (api().il2cpp_type_get_object)(self.ty().0) }
    }

    pub fn is_enum(self) -> bool {
        unsafe { (api().il2cpp_class_is_enum)(self.0) }
    }

    pub fn is_value_type(self) -> bool {
        unsafe { (api().il2cpp_class_is_valuetype)(self.0) }
    }

    pub fn value_size(self) -> usize {
        let mut align = 0;
        unsafe { (api().il2cpp_class_value_size)(self.0, &mut align) }.max(0) as usize
    }

    pub fn is_assignable_from(self, other: Self) -> bool {
        unsafe { (api().il2cpp_class_is_assignable_from)(self.0, other.0) }
    }

    pub fn field(self, name: &str) -> Option<Field> {
        let name = CString::new(name).ok()?;
        Field::from_ptr(unsafe { (api().il2cpp_class_get_field_from_name)(self.0, name.as_ptr()) })
    }

    pub fn field_any(self, names: &[&str]) -> Option<Field> {
        names.iter().find_map(|n| self.field(n))
    }

    pub fn instance_field(self, normalized: &str) -> Option<Field> {
        self.lineage()
            .flat_map(Self::fields)
            .find(|f| !f.is_static() && normalize(f.name()) == normalized)
    }

    pub fn fields(self) -> Vec<Field> {
        let mut iter: Ptr = ptr::null_mut();
        let mut out = Vec::new();
        loop {
            let field = unsafe { (api().il2cpp_class_get_fields)(self.0, &mut iter) };
            match Field::from_ptr(field) {
                Some(f) => out.push(f),
                None => return out,
            }
        }
    }

    pub fn methods(self) -> Vec<Method> {
        let mut iter: Ptr = ptr::null_mut();
        let mut out = Vec::new();
        loop {
            let method = unsafe { (api().il2cpp_class_get_methods)(self.0, &mut iter) };
            if method.is_null() {
                return out;
            }
            out.push(Method(method));
        }
    }

    pub fn method(self, name: &str, argc: i32) -> Option<Method> {
        let name = CString::new(name).ok()?;
        let method = unsafe { (api().il2cpp_class_get_method_from_name)(self.0, name.as_ptr(), argc) };
        (!method.is_null()).then_some(Method(method))
    }

    pub fn method_where(self, name: &str, accept: impl Fn(Method) -> bool) -> Option<Method> {
        self.lineage()
            .flat_map(Self::methods)
            .find(|m| m.name() == name && accept(*m))
    }

    pub fn new_object(self) -> Ptr {
        unsafe { (api().il2cpp_object_new)(self.0) }
    }

    pub fn enum_values(self) -> Vec<(String, i64)> {
        let Some(prim) = self
            .fields()
            .iter()
            .find(|f| !f.is_static())
            .and_then(Field::prim)
        else {
            return Vec::new();
        };
        self.fields()
            .into_iter()
            .filter(|f| f.is_literal())
            .map(|f| {
                let mut raw = [0u8; 8];
                unsafe { (api().il2cpp_field_static_get_value)(f.0, raw.as_mut_ptr().cast()) };
                (f.name().to_owned(), unsafe { prim.read_i64(raw.as_ptr()) })
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Type(pub Ptr);

impl Type {
    pub fn code(self) -> i32 {
        unsafe { (api().il2cpp_type_get_type)(self.0) }
    }

    pub fn class(self) -> Option<Class> {
        Class::from_ptr(unsafe { (api().il2cpp_class_from_type)(self.0) })
    }

    pub fn name(self) -> String {
        let raw = unsafe { (api().il2cpp_type_get_name)(self.0) };
        if raw.is_null() {
            return String::new();
        }
        let name = text(raw).to_owned();
        unsafe { (api().il2cpp_free)(raw.cast()) };
        name
    }

    pub fn is_reference(self) -> bool {
        matches!(self.code(), 0x0e | 0x12 | 0x14 | 0x1c | 0x1d) || (self.code() == 0x15 && !self.is_value())
    }

    pub fn is_value(self) -> bool {
        self.class().is_some_and(Class::is_value_type)
    }

    pub fn prim(self) -> Option<Prim> {
        match self.code() {
            0x02 => Some(Prim::Bool),
            0x03 | 0x07 => Some(Prim::U16),
            0x04 => Some(Prim::I8),
            0x05 => Some(Prim::U8),
            0x06 => Some(Prim::I16),
            0x08 => Some(Prim::I32),
            0x09 => Some(Prim::U32),
            0x0a => Some(Prim::I64),
            0x0b => Some(Prim::U64),
            0x0c => Some(Prim::F32),
            0x0d => Some(Prim::F64),
            0x18 | 0x19 => Some(if size_of::<usize>() == 8 {
                Prim::I64
            } else {
                Prim::I32
            }),
            0x11 => {
                let class = self.class()?;
                if !class.is_enum() {
                    return None;
                }
                class.fields().iter().find(|f| !f.is_static())?.prim()
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prim {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl Prim {
    pub fn size(self) -> usize {
        match self {
            Self::Bool | Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }

    pub unsafe fn read_f64(self, at: *const u8) -> f64 {
        unsafe {
            match self {
                Self::F32 => f64::from(ptr::read_unaligned(at.cast::<f32>())),
                Self::F64 => ptr::read_unaligned(at.cast::<f64>()),
                _ => self.read_i64(at) as f64,
            }
        }
    }

    pub unsafe fn read_i64(self, at: *const u8) -> i64 {
        unsafe {
            match self {
                Self::Bool | Self::U8 => i64::from(ptr::read_unaligned(at)),
                Self::I8 => i64::from(ptr::read_unaligned(at.cast::<i8>())),
                Self::I16 => i64::from(ptr::read_unaligned(at.cast::<i16>())),
                Self::U16 => i64::from(ptr::read_unaligned(at.cast::<u16>())),
                Self::I32 => i64::from(ptr::read_unaligned(at.cast::<i32>())),
                Self::U32 => i64::from(ptr::read_unaligned(at.cast::<u32>())),
                Self::I64 | Self::U64 => ptr::read_unaligned(at.cast::<i64>()),
                Self::F32 => ptr::read_unaligned(at.cast::<f32>()) as i64,
                Self::F64 => ptr::read_unaligned(at.cast::<f64>()) as i64,
            }
        }
    }

    pub unsafe fn write(self, at: *mut u8, value: f64) {
        unsafe {
            match self {
                Self::Bool => ptr::write_unaligned(at, u8::from(value != 0.0)),
                Self::I8 => ptr::write_unaligned(at.cast::<i8>(), value as i8),
                Self::U8 => ptr::write_unaligned(at, value as u8),
                Self::I16 => ptr::write_unaligned(at.cast::<i16>(), value as i16),
                Self::U16 => ptr::write_unaligned(at.cast::<u16>(), value as u16),
                Self::I32 => ptr::write_unaligned(at.cast::<i32>(), value as i32),
                Self::U32 => ptr::write_unaligned(at.cast::<u32>(), value as u32),
                Self::I64 => ptr::write_unaligned(at.cast::<i64>(), value as i64),
                Self::U64 => ptr::write_unaligned(at.cast::<u64>(), value as u64),
                Self::F32 => ptr::write_unaligned(at.cast::<f32>(), value as f32),
                Self::F64 => ptr::write_unaligned(at.cast::<f64>(), value),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Field(pub Ptr);

unsafe impl Send for Field {}
unsafe impl Sync for Field {}

impl Field {
    fn from_ptr(ptr: Ptr) -> Option<Self> {
        (!ptr.is_null()).then_some(Self(ptr))
    }

    pub fn name(self) -> &'static str {
        text(unsafe { (api().il2cpp_field_get_name)(self.0) })
    }

    pub fn offset(self) -> usize {
        unsafe { (api().il2cpp_field_get_offset)(self.0) }
    }

    pub fn inline_offset(self) -> usize {
        self.offset().saturating_sub(HEADER)
    }

    pub fn ty(self) -> Type {
        Type(unsafe { (api().il2cpp_field_get_type)(self.0) })
    }

    pub fn flags(self) -> i32 {
        unsafe { (api().il2cpp_field_get_flags)(self.0) }
    }

    pub fn is_static(self) -> bool {
        self.flags() & FIELD_STATIC != 0
    }

    pub fn is_literal(self) -> bool {
        self.flags() & FIELD_LITERAL != 0
    }

    pub fn prim(&self) -> Option<Prim> {
        self.ty().prim()
    }

    pub fn static_ptr(self) -> Ptr {
        let mut value: Ptr = ptr::null_mut();
        unsafe { (api().il2cpp_field_static_get_value)(self.0, (&raw mut value).cast()) };
        value
    }

    pub fn static_i64(self) -> Option<i64> {
        let prim = self.prim()?;
        let mut raw = [0u8; 8];
        unsafe { (api().il2cpp_field_static_get_value)(self.0, raw.as_mut_ptr().cast()) };
        Some(unsafe { prim.read_i64(raw.as_ptr()) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Method(pub Ptr);

unsafe impl Send for Method {}
unsafe impl Sync for Method {}

impl Method {
    /// Instantiates a generic method definition through reflection. The
    /// definition's own code pointer is only IL2CPP's "not instantiated"
    /// stub; the instance points at the real (for reference types, shared)
    /// body that the game actually runs.
    pub fn inflate(self, owner: Class, args: &[Class], type_class: Class) -> Option<Self> {
        let reflected = unsafe { (api().il2cpp_method_get_object)(self.0, owner.0) };
        let make = Class::of(reflected)?.method_where("MakeGenericMethod", |m| m.param_count() == 1)?;
        let array = unsafe { (api().il2cpp_array_new)(type_class.0, args.len()) };
        if array.is_null() {
            return None;
        }
        let slots = super::collections::array_data(array).cast_mut().cast::<Ptr>();
        for (index, class) in args.iter().enumerate() {
            unsafe { slots.add(index).write(class.type_object()) };
        }
        let inflated = make.invoke(reflected, &mut [array]).ok()?;
        let method = unsafe { (api().il2cpp_method_get_from_reflection)(inflated) };
        (!method.is_null()).then_some(Self(method))
    }

    pub fn name(self) -> &'static str {
        text(unsafe { (api().il2cpp_method_get_name)(self.0) })
    }

    pub fn param_count(self) -> u32 {
        unsafe { (api().il2cpp_method_get_param_count)(self.0) }
    }

    pub fn param(self, index: u32) -> Type {
        Type(unsafe { (api().il2cpp_method_get_param)(self.0, index) })
    }

    pub fn is_static(self) -> bool {
        let mut iflags = 0;
        (unsafe { (api().il2cpp_method_get_flags)(self.0, &mut iflags) } & 0x10) != 0
    }

    pub fn code(self) -> *const () {
        unsafe { ptr::read(self.0.cast::<*const ()>()) }
    }

    pub fn invoke(self, instance: Ptr, args: &mut [Ptr]) -> Result<Ptr, String> {
        let mut exception: Ptr = ptr::null_mut();
        let params = if args.is_empty() {
            ptr::null_mut()
        } else {
            args.as_mut_ptr()
        };
        let result = unsafe { (api().il2cpp_runtime_invoke)(self.0, instance, params, &mut exception) };
        match Class::of(exception) {
            Some(class) => Err(class.full_name()),
            None => Ok(result),
        }
    }
}
