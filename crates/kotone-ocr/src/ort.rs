//! 最小 ONNX Runtime C API 绑定（仅 `onnx` feature）。
//!
//! 运行时来源：sherpa-onnx v1.13.5 Windows 静态预编译包内的 `onnxruntime.lib`
//! （ONNX Runtime 1.27.1，CPU）。本 crate 不新增原生库，与 ASR 共享同一份运行时。
//!
//! OrtApi 是只追加的函数指针表；下列下标取自官方 v1.27.1 头文件
//! `include/onnxruntime/core/session/onnxruntime_c_api.h`（ORT_API_VERSION=27）中
//! `struct OrtApi` 的声明顺序，并在 docs/research/game-tools-m2-implementation.md 记录核对方法。
//! 运行时若不支持 API 27（`GetApi` 返回 NULL）则整个本地后端报告不可用，而不是猜测布局。

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;
use std::sync::OnceLock;

pub const ORT_API_VERSION: u32 = 27;

#[repr(C)]
struct OrtApiBase {
    get_api: unsafe extern "system" fn(u32) -> *const c_void,
    get_version_string: unsafe extern "system" fn() -> *const c_char,
}

extern "system" {
    fn OrtGetApiBase() -> *const OrtApiBase;
}

type Status = *mut c_void;

// OrtApi 函数表下标（v1.27.1 头文件声明顺序）。
mod idx {
    pub const GET_ERROR_MESSAGE: usize = 2;
    pub const CREATE_ENV: usize = 3;
    pub const CREATE_SESSION_FROM_ARRAY: usize = 8;
    pub const RUN: usize = 9;
    pub const CREATE_SESSION_OPTIONS: usize = 10;
    pub const DISABLE_CPU_MEM_ARENA: usize = 19;
    pub const SET_GRAPH_OPT_LEVEL: usize = 23;
    pub const SET_INTRA_OP_THREADS: usize = 24;
    pub const SET_INTER_OP_THREADS: usize = 25;
    pub const SESSION_GET_INPUT_NAME: usize = 36;
    pub const SESSION_GET_OUTPUT_NAME: usize = 37;
    pub const CREATE_RUN_OPTIONS: usize = 39;
    pub const RUN_OPTIONS_SET_TERMINATE: usize = 46;
    pub const CREATE_TENSOR_WITH_DATA: usize = 49;
    pub const GET_TENSOR_MUTABLE_DATA: usize = 51;
    pub const GET_DIMENSIONS_COUNT: usize = 61;
    pub const GET_DIMENSIONS: usize = 62;
    pub const GET_TENSOR_TYPE_AND_SHAPE: usize = 65;
    pub const CREATE_CPU_MEMORY_INFO: usize = 69;
    pub const ALLOCATOR_FREE: usize = 76;
    pub const GET_ALLOCATOR_WITH_DEFAULT_OPTIONS: usize = 78;
    pub const RELEASE_STATUS: usize = 93;
    pub const RELEASE_SESSION: usize = 95;
    pub const RELEASE_VALUE: usize = 96;
    pub const RELEASE_RUN_OPTIONS: usize = 97;
    pub const RELEASE_TENSOR_TYPE_AND_SHAPE_INFO: usize = 99;
    pub const RELEASE_SESSION_OPTIONS: usize = 100;
}

const ORT_LOGGING_LEVEL_ERROR: i32 = 3;
const ORT_ENABLE_ALL: i32 = 99;
const ORT_ARENA_ALLOCATOR: i32 = 1;
const ORT_MEM_TYPE_DEFAULT: i32 = 0;
const TENSOR_FLOAT: i32 = 1;

struct Api {
    table: *const *const c_void,
    version: String,
    env: *mut c_void,
    memory_info: *mut c_void,
}

// ORT 的 API 表、环境与 CPU MemoryInfo 是进程级只读对象，可跨线程共享。
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

static API: OnceLock<Result<Api, String>> = OnceLock::new();

macro_rules! f {
    ($api:expr, $idx:expr, fn($($arg:ty),* $(,)?) -> $ret:ty) => {{
        let ptr = *$api.table.add($idx);
        std::mem::transmute::<*const c_void, unsafe extern "system" fn($($arg),*) -> $ret>(ptr)
    }};
}

impl Api {
    unsafe fn check(&self, status: Status) -> Result<(), String> {
        if status.is_null() {
            return Ok(());
        }
        let msg = f!(self, idx::GET_ERROR_MESSAGE, fn(Status) -> *const c_char)(status);
        let text = if msg.is_null() {
            "unknown onnxruntime error".to_string()
        } else {
            CStr::from_ptr(msg).to_string_lossy().into_owned()
        };
        f!(self, idx::RELEASE_STATUS, fn(Status) -> ())(status);
        Err(text)
    }
}

fn ort_api() -> Result<&'static Api, String> {
    API.get_or_init(|| unsafe {
        let base = OrtGetApiBase();
        if base.is_null() {
            return Err("OrtGetApiBase 返回 NULL".into());
        }
        let version_ptr = ((*base).get_version_string)();
        let version = if version_ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(version_ptr).to_string_lossy().into_owned()
        };
        let table = ((*base).get_api)(ORT_API_VERSION) as *const *const c_void;
        if table.is_null() {
            return Err(format!(
                "ONNX Runtime {version} 不支持 C API 版本 {ORT_API_VERSION}"
            ));
        }
        let mut api = Api {
            table,
            version,
            env: ptr::null_mut(),
            memory_info: ptr::null_mut(),
        };
        let logid = CString::new("kotone-ocr").unwrap();
        let mut env = ptr::null_mut();
        api.check(f!(
            api,
            idx::CREATE_ENV,
            fn(i32, *const c_char, *mut *mut c_void) -> Status
        )(ORT_LOGGING_LEVEL_ERROR, logid.as_ptr(), &mut env))?;
        api.env = env;
        let mut info = ptr::null_mut();
        api.check(f!(
            api,
            idx::CREATE_CPU_MEMORY_INFO,
            fn(i32, i32, *mut *mut c_void) -> Status
        )(
            ORT_ARENA_ALLOCATOR, ORT_MEM_TYPE_DEFAULT, &mut info
        ))?;
        api.memory_info = info;
        Ok(api)
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// 运行时版本（例如 "1.27.1"）；不可用时返回错误原因。
pub fn runtime_version() -> Result<String, String> {
    ort_api().map(|a| a.version.clone())
}

/// 一个推理会话（模型字节在内存中加载，加载前已完成哈希校验）。
pub struct Session {
    raw: *mut c_void,
    input: CString,
    output: CString,
}

// OrtSession::Run 对同一会话的并发调用是线程安全的（ORT 文档约定）。
unsafe impl Send for Session {}
unsafe impl Sync for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(api) = ort_api() {
            unsafe { f!(api, idx::RELEASE_SESSION, fn(*mut c_void) -> ())(self.raw) };
        }
    }
}

/// 输出张量（拷贝到 Rust 内存）。
pub struct Tensor {
    pub shape: Vec<i64>,
    pub data: Vec<f32>,
}

/// 可从其他线程触发的终止句柄（RunOptionsSetTerminate）。
#[derive(Clone, Copy)]
pub struct Terminator(*mut c_void);
unsafe impl Send for Terminator {}
unsafe impl Sync for Terminator {}

impl Terminator {
    pub fn terminate(&self) {
        if let Ok(api) = ort_api() {
            unsafe {
                let status = f!(
                    api,
                    idx::RUN_OPTIONS_SET_TERMINATE,
                    fn(*mut c_void) -> Status
                )(self.0);
                let _ = api.check(status);
            }
        }
    }
}

unsafe fn name(api: &Api, session: *mut c_void, index_fn: usize) -> Result<CString, String> {
    let mut allocator = ptr::null_mut();
    api.check(f!(
        api,
        idx::GET_ALLOCATOR_WITH_DEFAULT_OPTIONS,
        fn(*mut *mut c_void) -> Status
    )(&mut allocator))?;
    let mut raw: *mut c_char = ptr::null_mut();
    api.check(f!(
        api,
        index_fn,
        fn(*mut c_void, usize, *mut c_void, *mut *mut c_char) -> Status
    )(session, 0, allocator, &mut raw))?;
    let owned = CStr::from_ptr(raw).to_owned();
    api.check(f!(
        api,
        idx::ALLOCATOR_FREE,
        fn(*mut c_void, *mut c_void) -> Status
    )(allocator, raw as *mut c_void))?;
    Ok(owned)
}

impl Session {
    pub fn from_bytes(
        model: &[u8],
        intra_threads: i32,
        inter_threads: i32,
    ) -> Result<Self, String> {
        let api = ort_api()?;
        unsafe {
            let mut options = ptr::null_mut();
            api.check(f!(
                api,
                idx::CREATE_SESSION_OPTIONS,
                fn(*mut *mut c_void) -> Status
            )(&mut options))?;
            struct Opts(*mut c_void);
            impl Drop for Opts {
                fn drop(&mut self) {
                    if let Ok(api) = ort_api() {
                        unsafe {
                            f!(api, idx::RELEASE_SESSION_OPTIONS, fn(*mut c_void) -> ())(self.0)
                        };
                    }
                }
            }
            let options = Opts(options);
            api.check(f!(
                api,
                idx::SET_INTRA_OP_THREADS,
                fn(*mut c_void, i32) -> Status
            )(options.0, intra_threads))?;
            api.check(f!(
                api,
                idx::SET_INTER_OP_THREADS,
                fn(*mut c_void, i32) -> Status
            )(options.0, inter_threads))?;
            api.check(f!(
                api,
                idx::SET_GRAPH_OPT_LEVEL,
                fn(*mut c_void, i32) -> Status
            )(options.0, ORT_ENABLE_ALL))?;
            // 动态输入尺寸下关闭 CPU arena，避免内存只增不减（与 RapidOCR 默认一致）
            api.check(f!(
                api,
                idx::DISABLE_CPU_MEM_ARENA,
                fn(*mut c_void) -> Status
            )(options.0))?;
            let mut session = ptr::null_mut();
            api.check(f!(
                api,
                idx::CREATE_SESSION_FROM_ARRAY,
                fn(*mut c_void, *const c_void, usize, *mut c_void, *mut *mut c_void) -> Status
            )(
                api.env,
                model.as_ptr() as *const c_void,
                model.len(),
                options.0,
                &mut session,
            ))?;
            // 先由守卫持有会话：取名失败时随守卫释放
            let mut owned = Session {
                raw: session,
                input: CString::default(),
                output: CString::default(),
            };
            owned.input = name(api, session, idx::SESSION_GET_INPUT_NAME)?;
            owned.output = name(api, session, idx::SESSION_GET_OUTPUT_NAME)?;
            Ok(owned)
        }
    }

    /// 运行单输入单输出模型。`register` 在推理开始前收到可跨线程调用的终止句柄。
    pub fn run(
        &self,
        input: &mut [f32],
        shape: &[i64],
        register: impl FnOnce(Terminator) -> Box<dyn std::any::Any>,
    ) -> Result<Tensor, String> {
        let api = ort_api()?;
        unsafe {
            let mut run_options = ptr::null_mut();
            api.check(f!(
                api,
                idx::CREATE_RUN_OPTIONS,
                fn(*mut *mut c_void) -> Status
            )(&mut run_options))?;
            struct RunOpts(*mut c_void);
            impl Drop for RunOpts {
                fn drop(&mut self) {
                    if let Ok(api) = ort_api() {
                        unsafe { f!(api, idx::RELEASE_RUN_OPTIONS, fn(*mut c_void) -> ())(self.0) };
                    }
                }
            }
            let run_options = RunOpts(run_options);
            struct Value(*mut c_void);
            impl Drop for Value {
                fn drop(&mut self) {
                    if !self.0.is_null() {
                        if let Ok(api) = ort_api() {
                            unsafe { f!(api, idx::RELEASE_VALUE, fn(*mut c_void) -> ())(self.0) };
                        }
                    }
                }
            }
            let mut value = ptr::null_mut();
            api.check(f!(
                api,
                idx::CREATE_TENSOR_WITH_DATA,
                fn(
                    *mut c_void,
                    *mut c_void,
                    usize,
                    *const i64,
                    usize,
                    i32,
                    *mut *mut c_void,
                ) -> Status
            )(
                api.memory_info,
                input.as_mut_ptr() as *mut c_void,
                std::mem::size_of_val(input),
                shape.as_ptr(),
                shape.len(),
                TENSOR_FLOAT,
                &mut value,
            ))?;
            let input_value = Value(value);
            // 注册守卫先于 RunOptions 释放（字段逆序析构：registration 声明在后，先析构）
            let registration = register(Terminator(run_options.0));
            let mut output = ptr::null_mut();
            let in_names = [self.input.as_ptr()];
            let out_names = [self.output.as_ptr()];
            let in_values = [input_value.0 as *const c_void];
            let status = f!(
                api,
                idx::RUN,
                fn(
                    *mut c_void,
                    *mut c_void,
                    *const *const c_char,
                    *const *const c_void,
                    usize,
                    *const *const c_char,
                    usize,
                    *mut *mut c_void,
                ) -> Status
            )(
                self.raw,
                run_options.0,
                in_names.as_ptr(),
                in_values.as_ptr(),
                1,
                out_names.as_ptr(),
                1,
                &mut output,
            );
            drop(registration);
            api.check(status)?;
            let output = Value(output);
            let mut info = ptr::null_mut();
            api.check(f!(
                api,
                idx::GET_TENSOR_TYPE_AND_SHAPE,
                fn(*mut c_void, *mut *mut c_void) -> Status
            )(output.0, &mut info))?;
            struct Info(*mut c_void);
            impl Drop for Info {
                fn drop(&mut self) {
                    if let Ok(api) = ort_api() {
                        unsafe {
                            f!(
                                api,
                                idx::RELEASE_TENSOR_TYPE_AND_SHAPE_INFO,
                                fn(*mut c_void) -> ()
                            )(self.0)
                        };
                    }
                }
            }
            let info = Info(info);
            let mut count = 0usize;
            api.check(f!(
                api,
                idx::GET_DIMENSIONS_COUNT,
                fn(*mut c_void, *mut usize) -> Status
            )(info.0, &mut count))?;
            let mut dims = vec![0i64; count];
            api.check(f!(
                api,
                idx::GET_DIMENSIONS,
                fn(*mut c_void, *mut i64, usize) -> Status
            )(info.0, dims.as_mut_ptr(), count))?;
            let elements = dims.iter().try_fold(1usize, |acc, &d| {
                usize::try_from(d).ok().and_then(|d| acc.checked_mul(d))
            });
            let Some(elements) = elements else {
                return Err(format!("输出形状无效: {dims:?}"));
            };
            let mut data_ptr = ptr::null_mut();
            api.check(f!(
                api,
                idx::GET_TENSOR_MUTABLE_DATA,
                fn(*mut c_void, *mut *mut c_void) -> Status
            )(output.0, &mut data_ptr))?;
            let data = std::slice::from_raw_parts(data_ptr as *const f32, elements).to_vec();
            Ok(Tensor { shape: dims, data })
        }
    }
}
