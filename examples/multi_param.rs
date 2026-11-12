//! 编译期 + 运行时回归示例：验证无参 / 单参 / 多参三种 `#[register_hook]` 展开均可编译，
//! 且多参 before hook 能修改各参数克隆并把修改后的值透传到原始实现。
use hook_macro::register_hook;
use safer_ffi::ffi_export;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

/// 无参函数（回归）。
#[register_hook]
fn tick() -> i32 {
    0
}

/// 单参函数（回归）。
#[register_hook]
fn add_one(x: &i32) -> i32 {
    x + 1
}

/// 多参函数（新能力）。
#[register_hook]
fn add(a: &i32, b: &i32) -> i32 {
    a + b
}

unsafe extern "C" fn bump(a: *mut i32, b: *mut i32) -> bool {
    if let Some(v) = a.as_mut() {
        *v += 10;
    }
    if let Some(v) = b.as_mut() {
        *v += 20;
    }
    false
}

fn main() {
    unsafe {
        let _ = tick();
        let _ = add_one(&1);

        register_before_add(bump);
        // a: 1 -> 11, b: 2 -> 22, add = 33
        let result = add(&1, &2);
        assert_eq!(
            result, 33,
            "multi-parameter hook should forward mutated clones"
        );
    }
}
