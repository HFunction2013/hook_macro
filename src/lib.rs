use proc_macro::TokenStream;
use quote::quote;
use syn::LitStr;
use syn::{
    FnArg, ItemFn, PatType, ReturnType, Type, parse::Parse, parse::ParseStream, parse_macro_input,
};

/// 解析 fallback 参数的辅助结构
struct HookAttr {
    fallback: Option<String>,
    suffix: Option<String>,
    prefix: Option<String>,
    mangling: Option<String>,
}

impl Parse for HookAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut attr = HookAttr { fallback: None, suffix: None, prefix: None, mangling: None };

        while !input.is_empty() {
            let name: syn::Ident = input.parse()?;
            let name_str = name.to_string();

            match name_str.as_str() {
                "fallback" | "suffix" | "prefix" | "mangling" => {
                    let _: syn::Token![=] = input.parse()?;
                    let value: LitStr = input.parse()?;

                    match name_str.as_str() {
                        "fallback" => attr.fallback = Some(value.value()),
                        "suffix" => attr.suffix = Some(value.value()),
                        "prefix" => attr.prefix = Some(value.value()),
                        "mangling" => attr.mangling = Some(value.value()),
                        _ => unreachable!(),
                    }
                }
                _ => {
                    return Err(syn::Error::new(
                        name.span(),
                        format!(
                            "expected `fallback`, `suffix`, `prefix` or `mangling`, got `{}`",
                            name_str
                        ),
                    ));
                }
            }

            // 可选逗号分隔
            if !input.is_empty() && input.peek(syn::Token![,]) {
                let _: syn::Token![,] = input.parse()?;
            }
        }

        Ok(attr)
    }
}

/// 计算原始函数的重命名名称
fn compute_orig_name(fn_name: &syn::Ident, attr: &HookAttr) -> (String, syn::Ident) {
    let fn_name_str = fn_name.to_string();

    let orig_name_str = if let Some(ref mangling) = attr.mangling {
        // 直接指定完整名称
        mangling.clone()
    } else {
        // 使用 prefix + 原名 + suffix
        let prefix = attr.prefix.as_deref().unwrap_or("");
        let suffix = attr.suffix.as_deref().unwrap_or("_impl"); // 默认后缀
        format!("{}{}{}", prefix, fn_name_str, suffix)
    };

    let orig_name = syn::Ident::new(&orig_name_str, fn_name.span());
    (orig_name_str, orig_name)
}

#[proc_macro_attribute]
pub fn register_hook(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);

    // 解析属性参数
    let hook_attr = parse_macro_input!(attr as HookAttr);

    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();

    // 计算原始函数的重命名名称
    let (_orig_name_str, orig_fn_name) = compute_orig_name(fn_name, &hook_attr);

    // wrapper 就是原名
    let wrapper_name = fn_name.clone();
    let wrapper_name_str = fn_name_str.clone();
    let fn_name_upper = fn_name_str.to_uppercase();

    let inputs = &input_fn.sig.inputs;
    let param_count = inputs.len();

    assert!(param_count <= 1, "Function with #[register_hook] must have at most one parameter");

    let (param_name, param_type, inner_type) = if param_count == 1 {
        match &inputs[0] {
            FnArg::Typed(PatType { pat, ty, .. }) => {
                let name = match pat.as_ref() {
                    syn::Pat::Ident(p) => &p.ident,
                    _ => panic!("Expected simple parameter name"),
                };

                let inner = match ty.as_ref() {
                    Type::Reference(type_ref) => &type_ref.elem,
                    other => other,
                };

                (Some(name.clone()), Some(ty.as_ref()), Some(inner))
            }
            _ => (None, None, None),
        }
    } else {
        (None, None, None)
    };

    let original_output = &input_fn.sig.output;

    generate_hook_system(
        &input_fn,
        &orig_fn_name,
        &wrapper_name,
        &wrapper_name_str,
        fn_name_upper,
        param_name,
        param_type,
        inner_type,
        original_output,
        param_count > 0,
        hook_attr.fallback.as_deref(), // 传入 fallback
    )
    .into()
}

#[allow(clippy::too_many_arguments)]
fn generate_hook_system(
    original_fn: &ItemFn,
    impl_fn_name: &syn::Ident,
    wrapper_fn_name: &syn::Ident,
    wrapper_name_str: &str,
    fn_name_upper: String,
    param_name: Option<syn::Ident>,
    param_type: Option<&Type>,
    inner_type: Option<&Type>,
    original_output: &ReturnType,
    has_param: bool,
    fallback_expr: Option<&str>,
) -> proc_macro2::TokenStream {
    let before_hook_name = syn::Ident::new(
        &format!("Before{}Hook", to_upper_camel_case(wrapper_name_str)),
        wrapper_fn_name.span(),
    );
    let on_hook_name = syn::Ident::new(
        &format!("On{}Hook", to_upper_camel_case(wrapper_name_str)),
        wrapper_fn_name.span(),
    );
    let after_hook_name = syn::Ident::new(
        &format!("After{}Hook", to_upper_camel_case(wrapper_name_str)),
        wrapper_fn_name.span(),
    );

    let before_hooks_static =
        syn::Ident::new(&format!("BEFORE_{}_HOOKS", fn_name_upper), wrapper_fn_name.span());
    let on_hooks_static =
        syn::Ident::new(&format!("ON_{}_HOOKS", fn_name_upper), wrapper_fn_name.span());
    let after_hooks_static =
        syn::Ident::new(&format!("AFTER_{}_HOOKS", fn_name_upper), wrapper_fn_name.span());
    let in_flag_static = syn::Ident::new(&format!("IN_{}", fn_name_upper), wrapper_fn_name.span());

    let register_before =
        syn::Ident::new(&format!("register_before_{}", wrapper_name_str), wrapper_fn_name.span());
    let register_on =
        syn::Ident::new(&format!("register_on_{}", wrapper_name_str), wrapper_fn_name.span());
    let register_after =
        syn::Ident::new(&format!("register_after_{}", wrapper_name_str), wrapper_fn_name.span());
    let clear_hooks =
        syn::Ident::new(&format!("clear_{}_hooks", wrapper_name_str), wrapper_fn_name.span());

    let run_before =
        syn::Ident::new(&format!("run_before_{}_chain", wrapper_name_str), wrapper_fn_name.span());
    let run_on =
        syn::Ident::new(&format!("run_on_{}_chain", wrapper_name_str), wrapper_fn_name.span());
    let run_after =
        syn::Ident::new(&format!("run_after_{}_chain", wrapper_name_str), wrapper_fn_name.span());

    let fn_block = &original_fn.block;
    let fn_vis = &original_fn.vis;
    let fn_attrs = &original_fn.attrs;

    let default_result = if let Some(expr) = fallback_expr {
        let expr_token: proc_macro2::TokenStream =
            expr.parse().expect("Invalid fallback expression");
        quote! { #expr_token }
    } else {
        quote! { Default::default() }
    };

    if has_param {
        let p_name = param_name.as_ref().unwrap();
        let p_type = param_type.unwrap();
        let p_inner = inner_type.unwrap();

        quote! {
            #(#fn_attrs)*
            #fn_vis fn #impl_fn_name(#p_name: #p_type) #original_output #fn_block

            type #before_hook_name = unsafe extern "C" fn(*mut #p_inner) -> bool;
            type #on_hook_name = unsafe extern "C" fn(*const #p_inner) -> bool;
            type #after_hook_name = unsafe extern "C" fn(*const #p_inner) -> bool;

            static #before_hooks_static: LazyLock<Mutex<Vec<#before_hook_name>>> =
                LazyLock::new(|| Mutex::new(Vec::new()));
            static #on_hooks_static: LazyLock<Mutex<Vec<#on_hook_name>>> =
                LazyLock::new(|| Mutex::new(Vec::new()));
            static #after_hooks_static: LazyLock<Mutex<Vec<#after_hook_name>>> =
                LazyLock::new(|| Mutex::new(Vec::new()));

            static #in_flag_static: AtomicBool = AtomicBool::new(false);

            #[ffi_export]
            pub fn #register_before(hook: #before_hook_name) {
                if let Ok(mut hooks) = #before_hooks_static.lock() {
                    hooks.push(hook);
                }
            }

            #[ffi_export]
            pub fn #register_on(hook: #on_hook_name) {
                if let Ok(mut hooks) = #on_hooks_static.lock() {
                    hooks.push(hook);
                }
            }

            #[ffi_export]
            pub fn #register_after(hook: #after_hook_name) {
                if let Ok(mut hooks) = #after_hooks_static.lock() {
                    hooks.push(hook);
                }
            }

            #[ffi_export]
            pub fn #clear_hooks() {
                let _ = #before_hooks_static.lock().map(|mut h| h.clear());
                let _ = #on_hooks_static.lock().map(|mut h| h.clear());
                let _ = #after_hooks_static.lock().map(|mut h| h.clear());
            }

            unsafe fn #run_before(param: *mut #p_inner) -> bool {
                if let Ok(list) = #before_hooks_static.lock() {
                    for h in list.iter() {
                        unsafe {
                            if h(param) {
                                return true;
                            }
                        }
                    }
                }
                false
            }

            unsafe fn #run_on(param: *const #p_inner) -> bool {
                if let Ok(list) = #on_hooks_static.lock() {
                    for h in list.iter() {
                        unsafe {
                            if h(param) {
                                return true;
                            }
                        }
                    }
                }
                false
            }

            unsafe fn #run_after(param: *const #p_inner) -> bool {
                if let Ok(list) = #after_hooks_static.lock() {
                    for h in list.iter() {
                        unsafe {
                            if h(param) {
                                return true;
                            }
                        }
                    }
                }
                false
            }

            #[ffi_export]
            pub unsafe fn #wrapper_fn_name(#p_name: #p_type) #original_output {
                let reenter = #in_flag_static.load(Ordering::SeqCst);
                if reenter {
                    return #impl_fn_name(#p_name);
                }
                #in_flag_static.store(true, Ordering::SeqCst);

                // Clone 一份数据，BeforeHook 可以修改这份克隆
                let mut data = (#p_name).clone();
                let param_ptr: *mut #p_inner = &mut data;

                let mut interrupted = unsafe { #run_before(param_ptr) };

                if !interrupted {
                    // 用修改后的数据继续往后传
                    let data_ptr: *const #p_inner = &data;
                    interrupted = unsafe { #run_on(data_ptr) };
                }

                let result = if !interrupted {
                    // 原始函数也接收修改后的数据
                    let res = #impl_fn_name(&data);
                    let data_ptr: *const #p_inner = &data;
                    unsafe { #run_after(data_ptr) };
                    res
                } else {
                    #default_result
                };

                #in_flag_static.store(false, Ordering::SeqCst);
                result
            }
        }
    } else {
        quote! {
            #(#fn_attrs)*
            #fn_vis fn #impl_fn_name() #original_output #fn_block

            type #before_hook_name = unsafe extern "C" fn() -> bool;
            type #on_hook_name = unsafe extern "C" fn() -> bool;
            type #after_hook_name = unsafe extern "C" fn() -> bool;

            static #before_hooks_static: LazyLock<Mutex<Vec<#before_hook_name>>> =
                LazyLock::new(|| Mutex::new(Vec::new()));
            static #on_hooks_static: LazyLock<Mutex<Vec<#on_hook_name>>> =
                LazyLock::new(|| Mutex::new(Vec::new()));
            static #after_hooks_static: LazyLock<Mutex<Vec<#after_hook_name>>> =
                LazyLock::new(|| Mutex::new(Vec::new()));

            static #in_flag_static: AtomicBool = AtomicBool::new(false);

            #[ffi_export]
            pub fn #register_before(hook: #before_hook_name) {
                if let Ok(mut hooks) = #before_hooks_static.lock() {
                    hooks.push(hook);
                }
            }

            #[ffi_export]
            pub fn #register_on(hook: #on_hook_name) {
                if let Ok(mut hooks) = #on_hooks_static.lock() {
                    hooks.push(hook);
                }
            }

            #[ffi_export]
            pub fn #register_after(hook: #after_hook_name) {
                if let Ok(mut hooks) = #after_hooks_static.lock() {
                    hooks.push(hook);
                }
            }

            #[ffi_export]
            pub fn #clear_hooks() {
                let _ = #before_hooks_static.lock().map(|mut h| h.clear());
                let _ = #on_hooks_static.lock().map(|mut h| h.clear());
                let _ = #after_hooks_static.lock().map(|mut h| h.clear());
            }

            unsafe fn #run_before() -> bool {
                if let Ok(list) = #before_hooks_static.lock() {
                    for h in list.iter() {
                        unsafe {
                            if h() {
                                return true;
                            }
                        }
                    }
                }
                false
            }

            unsafe fn #run_on() -> bool {
                if let Ok(list) = #on_hooks_static.lock() {
                    for h in list.iter() {
                        unsafe {
                            if h() {
                                return true;
                            }
                        }
                    }
                }
                false
            }

            unsafe fn #run_after() -> bool {
                if let Ok(list) = #after_hooks_static.lock() {
                    for h in list.iter() {
                        unsafe {
                            if h() {
                                return true;
                            }
                        }
                    }
                }
                false
            }

            #[ffi_export]
            pub unsafe fn #wrapper_fn_name() #original_output {
                let reenter = #in_flag_static.load(Ordering::SeqCst);
                if reenter {
                    return #impl_fn_name();
                }
                #in_flag_static.store(true, Ordering::SeqCst);

                let mut interrupted = unsafe { #run_before() };

                if !interrupted {
                    interrupted = unsafe { #run_on() };
                }

                let result = if !interrupted {
                    let res = #impl_fn_name();
                    unsafe { #run_after() };
                    res
                } else {
                    #default_result
                };

                #in_flag_static.store(false, Ordering::SeqCst);
                result
            }
        }
    }
}

/// 下划线命名转大驼峰命名
fn to_upper_camel_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect()
}
