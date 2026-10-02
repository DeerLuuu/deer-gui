//! 共享的「运行时动态加载 Vulkan loader」能力。
//!
//! ## 为什么必须动态加载
//!
//! M1 实测：`#[link(name = "vulkan-1")]` 在**没装 SDK** 的机器上链接失败 ——
//! `LNK1181: 无法打开输入文件 "vulkan-1.lib"`。系统只带 `vulkan-1.dll`（loader），
//! 而导入库 `.lib` 属于 Vulkan SDK。所以走 `LoadLibraryW` + `GetProcAddress`，
//! 只依赖 `kernel32`。
//!
//! `ffi.rs` 里那份是内联实现（已验证）；本模块把它抽出来给**设备级**复用，
//! 避免两处各写一遍 `LoadLibraryW`。

use std::ffi::{c_char, c_void};

use deer_core::{ GpuError, GpuResult };

pub type Module = *mut c_void;
pub type FARPROC = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> Module;
    fn GetProcAddress(module: Module, name: *const c_char) -> FARPROC;
    fn FreeLibrary(module: Module) -> i32;
}

/// 一个已加载的 `vulkan-1.dll`。`Drop` 时 `FreeLibrary`。
pub struct Lib {
    module: Module,
}

// SAFETY: `HMODULE` 是进程级的、可跨线程使用的句柄；
// 我们在 `Drop` 里保证只 `FreeLibrary` 一次，且不并发调用。
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

impl Lib {
    /// 打开 `vulkan-1.dll`。不存在时返回 `Unsupported`（**不 panic**）——
    /// 上层可据此回退到 CPU 后端。
    pub fn open() -> GpuResult<Lib> {
        let name: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
        // SAFETY: 传以 NUL 结尾的 UTF-16 字符串；失败返回空句柄，我们会检查。
        let module = unsafe { LoadLibraryW(name.as_ptr()) };
        if module.is_null() {
            return Err(GpuError::Unsupported(
                "找不到 vulkan-1.dll（本机没有 Vulkan loader）⇒ 请改用 CPU 后端".to_string(),
            ));
        }
        Ok(Lib { module })
    }

    /// 取一个符号并转成函数指针。
    ///
    /// # Safety
    /// `T` 必须是该符号**真实签名**对应的函数指针类型。传错类型不会在编译期报错
    /// （这是 `transmute` 的固有代价），所以每个调用点都紧跟一条注释说明依据。
    pub unsafe fn sym<T: Copy>(&self, name: &str) -> GpuResult<T> {
        let mut cname = Vec::with_capacity(name.len() + 1);
        cname.extend_from_slice(name.as_bytes());
        cname.push(0);
        // SAFETY: `cname` 以 NUL 结尾且在本调用期间存活；GetProcAddress 只读它。
        let p = unsafe { GetProcAddress(self.module, cname.as_ptr() as *const c_char) };
        if p.is_null() {
            return Err(GpuError::Unsupported(format!(
                "vulkan-1.dll 缺少符号 {name}（loader 版本过旧？）"
            )));
        }
        assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<FARPROC>(),
            "函数指针大小必须与 FARPROC 一致"
        );
        // SAFETY: 调用方保证 `T` 与符号真实签名一致。
        Ok(unsafe { std::mem::transmute_copy::<FARPROC, T>(&p) })
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        if !self.module.is_null() {
            // SAFETY: 模块由 `LoadLibraryW` 加载，且只在 Drop 里释放一次。
            unsafe { FreeLibrary(self.module) };
            self.module = std::ptr::null_mut();
        }
    }
}

/// 把 C 字符串字面量（带 NUL）转成 `*const c_char`。
///
/// 用 `c"name"` 字面量即可，本函数只是让意图更清楚。
pub fn cstr(s: &'static std::ffi::CStr) -> *const c_char {
    s.as_ptr()
}
