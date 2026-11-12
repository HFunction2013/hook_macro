# hook_macro

[![crates.io](https://img.shields.io/crates/v/hook_macro.svg)](https://crates.io/crates/hook_macro)
[![docs.rs](https://docs.rs/hook_macro/badge.svg)](https://docs.rs/hook_macro)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org)

Procedural macro for generating runtime hook systems with before/on/after callbacks.

## Features

- 🔧 Automatically generates `before`, `on`, and `after` hook chains
- 🌐 Supports FFI export for C/C++ interop
- 🛡️ Reentrancy protection to prevent infinite loops
- 🎨 Customizable naming with `prefix`, `suffix`, and `mangling` options
- 💾 Configurable fallback return value when hooks interrupt execution

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
hook_macro = "0.3"
```

## Quick Start

```rust
use hook_macro::register_hook;

#[register_hook]
fn add_one(x: &i32) -> i32 {
    x + 1
}

fn main() {
    // Register hooks
    register_before_add_one(|x| {
        println!("Before: {}", unsafe { &*x });
        false // Return true to interrupt execution
    });

    register_on_add_one(|x| {
        println!("On: {}", unsafe { &*x });
        false
    });

    register_after_add_one(|x| {
        println!("After: {}", unsafe { &*x });
        false
    });

    // Call the wrapped function
    let result = unsafe { add_one(&5) };
    println!("Result: {}", result);
}
```

## Examples

### Multiple Parameters

```rust
use hook_macro::register_hook;

#[register_hook(fallback = "0")]
fn divide(a: &i32, b: &i32) -> i32 {
    a / b
}

fn main() {
    // A before hook can mutate each cloned argument; the mutated clones are
    // forwarded to the original implementation. Returning `true` interrupts
    // the chain and the wrapper returns the fallback value.
    let result = unsafe { divide(&10, &2) };
    assert_eq!(result, 5);
}
```

### Out Output Parameters

A parameter whose type is `Out<'_, T>` (an FFI out-pointer, e.g. from
`safer-ffi`) is treated specially: it is **not** cloned, **not** added to the
hook chain, and forwarded verbatim to the original implementation. Only the
remaining business parameters participate in before/on/after hooks.

```rust
use hook_macro::register_hook;
use safer_ffi::prelude::*;

#[register_hook]
fn add(a: &i32, out: Out<'_, i32>) {
    out.write(a + 10);
}
```

### Custom Naming

```rust
use hook_macro::register_hook;

// Rename the original implementation
#[register_hook(suffix = "_original")]
fn my_function(x: &i32) -> i32 {
    x * 2
}

// Or use custom prefix
#[register_hook(prefix = "impl_")]
fn another_function(x: &i32) -> i32 {
    x + 1
}

// Or fully control the name
#[register_hook(mangling = "custom_impl_name")]
fn third_function(x: &i32) -> i32 {
    x - 1
}
```

## Generated API

For a function `foo` with one parameter, the macro generates:

| Function | Description |
|----------|-------------|
| `register_before_foo(hook)` | Register a before hook |
| `register_on_foo(hook)` | Register an on hook |
| `register_after_foo(hook)` | Register an after hook |
| `clear_foo_hooks()` | Clear all hooks |

### Hook Types

With one parameter `T`:

```rust
type BeforeFooHook = unsafe extern "C" fn(*mut T) -> bool;
type OnFooHook = unsafe extern "C" fn(*const T) -> bool;
type AfterFooHook = unsafe extern "C" fn(*const T) -> bool;
```

With multiple parameters `T1, T2, ...`:

```rust
type BeforeFooHook = unsafe extern "C" fn(*mut T1, *mut T2, ...) -> bool;
type OnFooHook = unsafe extern "C" fn(*const T1, *const T2, ...) -> bool;
type AfterFooHook = unsafe extern "C" fn(*const T1, *const T2, ...) -> bool;
```

Without parameters:

```rust
type BeforeFooHook = unsafe extern "C" fn() -> bool;
type OnFooHook = unsafe extern "C" fn() -> bool;
type AfterFooHook = unsafe extern "C" fn() -> bool;
```

Return `true` from a hook to interrupt the chain and stop execution.

## Attribute Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `fallback` | string | `Default::default()` | Return value when hooks interrupt execution |
| `prefix` | string | `""` | Prefix for the original implementation name |
| `suffix` | string | `"_impl"` | Suffix for the original implementation name |
| `mangling` | string | None | Full custom name for the original implementation |

## Limitations

- Functions may have any number of parameters (zero, one, or many)
- Business parameters must implement `Clone`; `Out<'_, T>` output parameters are exempt and are passed through untouched
- Return type must implement `Default` (unless `fallback` is specified)

## License

This project is licensed under the LICENSE.