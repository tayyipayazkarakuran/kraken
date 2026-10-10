use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

pub type Ptr = *mut c_void;

macro_rules! api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
        pub struct Api {
            $(pub $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Api {
            unsafe fn load(module: HMODULE) -> Result<Self, &'static str> {
                Ok(Self {
                    $($name: {
                        let symbol = unsafe {
                            GetProcAddress(module, concat!(stringify!($name), "\0").as_ptr())
                        };
                        match symbol {
                            Some(f) => unsafe { std::mem::transmute::<_, _>(f) },
                            None => return Err(stringify!($name)),
                        }
                    },)*
                })
            }
        }
    };
}

api! {
    il2cpp_domain_get: fn() -> Ptr;
    il2cpp_domain_get_assemblies: fn(Ptr, *mut usize) -> *const Ptr;
    il2cpp_assembly_get_image: fn(Ptr) -> Ptr;
    il2cpp_image_get_name: fn(Ptr) -> *const c_char;
    il2cpp_image_get_class_count: fn(Ptr) -> usize;
    il2cpp_image_get_class: fn(Ptr, usize) -> Ptr;
    il2cpp_class_get_name: fn(Ptr) -> *const c_char;
    il2cpp_class_get_namespace: fn(Ptr) -> *const c_char;
    il2cpp_class_get_parent: fn(Ptr) -> Ptr;
    il2cpp_class_get_method_from_name: fn(Ptr, *const c_char, i32) -> Ptr;
    il2cpp_class_get_methods: fn(Ptr, *mut Ptr) -> Ptr;
    il2cpp_class_get_fields: fn(Ptr, *mut Ptr) -> Ptr;
    il2cpp_class_get_field_from_name: fn(Ptr, *const c_char) -> Ptr;
    il2cpp_class_get_type: fn(Ptr) -> Ptr;
    il2cpp_class_from_type: fn(Ptr) -> Ptr;
    il2cpp_class_is_enum: fn(Ptr) -> bool;
    il2cpp_class_is_valuetype: fn(Ptr) -> bool;
    il2cpp_class_value_size: fn(Ptr, *mut u32) -> i32;
    il2cpp_class_array_element_size: fn(Ptr) -> i32;
    il2cpp_class_get_element_class: fn(Ptr) -> Ptr;
    il2cpp_class_is_assignable_from: fn(Ptr, Ptr) -> bool;
    il2cpp_field_get_name: fn(Ptr) -> *const c_char;
    il2cpp_field_get_offset: fn(Ptr) -> usize;
    il2cpp_field_get_type: fn(Ptr) -> Ptr;
    il2cpp_field_get_flags: fn(Ptr) -> i32;
    il2cpp_field_static_get_value: fn(Ptr, *mut c_void);
    il2cpp_type_get_name: fn(Ptr) -> *mut c_char;
    il2cpp_type_get_type: fn(Ptr) -> i32;
    il2cpp_type_get_object: fn(Ptr) -> Ptr;
    il2cpp_free: fn(*mut c_void);
    il2cpp_method_get_name: fn(Ptr) -> *const c_char;
    il2cpp_method_get_param_count: fn(Ptr) -> u32;
    il2cpp_method_get_param: fn(Ptr, u32) -> Ptr;
    il2cpp_method_get_flags: fn(Ptr, *mut u32) -> u32;
    il2cpp_runtime_invoke: fn(Ptr, Ptr, *mut Ptr, *mut Ptr) -> Ptr;
    il2cpp_object_new: fn(Ptr) -> Ptr;
    il2cpp_object_get_class: fn(Ptr) -> Ptr;
    il2cpp_object_unbox: fn(Ptr) -> Ptr;
    il2cpp_string_new: fn(*const c_char) -> Ptr;
    il2cpp_string_length: fn(Ptr) -> i32;
    il2cpp_string_chars: fn(Ptr) -> *const u16;
    il2cpp_thread_attach: fn(Ptr) -> Ptr;
    il2cpp_method_get_object: fn(Ptr, Ptr) -> Ptr;
    il2cpp_method_get_from_reflection: fn(Ptr) -> Ptr;
    il2cpp_array_new: fn(Ptr, usize) -> Ptr;
}

static API: OnceLock<Api> = OnceLock::new();

pub fn api() -> &'static Api {
    API.get().expect("il2cpp api not loaded")
}

pub fn game_module() -> Option<HMODULE> {
    let name: Vec<u16> = "GameAssembly.dll\0".encode_utf16().collect();
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    (!module.is_null()).then_some(module)
}

pub fn load() -> Result<&'static Api, String> {
    if let Some(api) = API.get() {
        return Ok(api);
    }
    let module = game_module().ok_or_else(|| "GameAssembly.dll is not loaded".to_owned())?;
    let loaded = unsafe { Api::load(module) }.map_err(|name| format!("missing export {name}"))?;
    Ok(API.get_or_init(|| loaded))
}
