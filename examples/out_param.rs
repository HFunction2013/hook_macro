//! 编译期 + 运行时回归：业务参数与 `Out` 出参混合时，出参不进入 hook 链、
//! 不被克隆，wrapper 直接透传给原始实现并由实现写入缓冲。
use hook_macro::register_hook;
use safer_ffi::ffi_export;
use safer_ffi::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

/// 一个业务参（`&i32`，参与 hook 链）+ 一个 `Out` 出参（透传）。
#[register_hook]
fn add(a: &i32, out: Out<'_, i32>) {
    out.write(a + 10);
}

fn main() {
    let mut buf = 0i32;
    let out = Out::from(&mut buf);

    unsafe {
        add(&1, out);
    }

    assert_eq!(buf, 11);
}
