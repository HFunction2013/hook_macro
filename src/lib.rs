use proc_macro::TokenStream;
use quote::quote;
use syn::LitStr;
use syn::{
    parse::Parse, parse::ParseStream, parse_macro_input, FnArg, ItemFn, PatType, ReturnType, Type,
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
        let mut attr = HookAttr {
            fallback: None,
            suffix: None,
            prefix: None,
            mangling: None,
        };

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
                            "expected `fallback`, `suffix`, `prefix` or `mangling`, got `{name_str}`"
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

/// 被标记函数的一个参数。
struct Param {
    /// 参数名（如 `x`）。
    name: syn::Ident,
    /// 原始参数类型（如 `&String`）。
    ty: Box<Type>,
    /// 剥去一层引用后的内部类型（如 `String`）。
    inner: Box<Type>,
    /// 是否为 `Out<'_, T>` 出参：不进 hook 链、不克隆，wrapper 直接透传。
    is_out: bool,
}

/// 判断类型是否为 `Out<'_, T>`（兼容 `safer_ffi::Out<...>` 全路径写法）。
fn is_out_type(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|seg| seg.ident == "Out")
            .unwrap_or(false),
        _ => false,
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
        format!("{prefix}{fn_name_str}{suffix}")
    };

    let orig_name = syn::Ident::new(&orig_name_str, fn_name.span());
    (orig_name_str, orig_name)
}

/// 解析被标记函数的全部参数（忽略 `self` 接收器）。
fn parse_params(inputs: &syn::punctuated::Punctuated<FnArg, syn::Token![,]>) -> Vec<Param> {
    let mut out = Vec::new();
    for arg in inputs {
        match arg {
            FnArg::Typed(PatType { pat, ty, .. }) => {
                let name = match pat.as_ref() {
                    syn::Pat::Ident(p) => p.ident.clone(),
                    _ => panic!("Expected simple parameter name"),
                };
                let inner: Box<Type> = match ty.as_ref() {
                    Type::Reference(type_ref) => type_ref.elem.clone(),
                    other => Box::new(other.clone()),
                };
                let is_out = is_out_type(ty);
                out.push(Param {
                    name,
                    ty: ty.clone(),
                    inner,
                    is_out,
                });
            }
            FnArg::Receiver(_) => {}
        }
    }
    out
}

#[proc_macro_attribute]
/// 注册 hook 宏
///
/// 支持无参、单参与多参函数。多参时 before hook 接收各参数的可变克隆指针，
/// on/after hook 接收只读克隆指针。
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

    let params = parse_params(&input_fn.sig.inputs);

    let original_output = &input_fn.sig.output;

    generate_hook_system(
        &input_fn,
        &orig_fn_name,
        &wrapper_name,
        &wrapper_name_str,
        fn_name_upper,
        &params,
        original_output,
        hook_attr.fallback.as_deref(), // 传入 fallback
    )
    .into()
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::needless_pass_by_value
)]
fn generate_hook_system(
    original_fn: &ItemFn,
    impl_fn_name: &syn::Ident,
    wrapper_fn_name: &syn::Ident,
    wrapper_name_str: &str,
    fn_name_upper: String,
    params: &[Param],
    original_output: &ReturnType,
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

    let before_hooks_static = syn::Ident::new(
        &format!("BEFORE_{fn_name_upper}_HOOKS"),
        wrapper_fn_name.span(),
    );
    let on_hooks_static =
        syn::Ident::new(&format!("ON_{fn_name_upper}_HOOKS"), wrapper_fn_name.span());
    let after_hooks_static = syn::Ident::new(
        &format!("AFTER_{fn_name_upper}_HOOKS"),
        wrapper_fn_name.span(),
    );
    let in_flag_static = syn::Ident::new(&format!("IN_{fn_name_upper}"), wrapper_fn_name.span());

    let register_before = syn::Ident::new(
        &format!("register_before_{wrapper_name_str}"),
        wrapper_fn_name.span(),
    );
    let register_on = syn::Ident::new(
        &format!("register_on_{wrapper_name_str}"),
        wrapper_fn_name.span(),
    );
    let register_after = syn::Ident::new(
        &format!("register_after_{wrapper_name_str}"),
        wrapper_fn_name.span(),
    );
    let clear_hooks = syn::Ident::new(
        &format!("clear_{wrapper_name_str}_hooks"),
        wrapper_fn_name.span(),
    );

    let run_before = syn::Ident::new(
        &format!("run_before_{wrapper_name_str}_chain"),
        wrapper_fn_name.span(),
    );
    let run_on = syn::Ident::new(
        &format!("run_on_{wrapper_name_str}_chain"),
        wrapper_fn_name.span(),
    );
    let run_after = syn::Ident::new(
        &format!("run_after_{wrapper_name_str}_chain"),
        wrapper_fn_name.span(),
    );

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

    // 分组：业务参数（参与 before/on/after hook 链）与 `Out` 出参（直接透传给实现，
    // 不克隆、不进 hook 指针链）。
    let biz: Vec<&Param> = params.iter().filter(|p| !p.is_out).collect();
    let all_names: Vec<&syn::Ident> = params.iter().map(|p| &p.name).collect();
    let all_types: Vec<&Type> = params.iter().map(|p| p.ty.as_ref()).collect();

    let biz_names: Vec<&syn::Ident> = biz.iter().map(|p| &p.name).collect();
    let biz_inners: Vec<&Type> = biz.iter().map(|p| p.inner.as_ref()).collect();

    let span = wrapper_fn_name.span();
    let n_biz = biz.len();
    // 业务参数对应的本地克隆绑定与各阶段指针。
    let data_idents: Vec<syn::Ident> = (0..n_biz)
        .map(|i| syn::Ident::new(&format!("__hook_data_{i}"), span))
        .collect();
    let bp_idents: Vec<syn::Ident> = (0..n_biz)
        .map(|i| syn::Ident::new(&format!("__hook_bp_{i}"), span))
        .collect();
    let op_idents: Vec<syn::Ident> = (0..n_biz)
        .map(|i| syn::Ident::new(&format!("__hook_op_{i}"), span))
        .collect();
    let ap_idents: Vec<syn::Ident> = (0..n_biz)
        .map(|i| syn::Ident::new(&format!("__hook_ap_{i}"), span))
        .collect();

    // 调用原始实现的实参，严格保持参数原顺序：业务参传克隆引用，出参透传绑定名。
    let mut impl_args: Vec<proc_macro2::TokenStream> = Vec::with_capacity(params.len());
    let mut bi = 0usize;
    for p in params {
        if p.is_out {
            let name = &p.name;
            impl_args.push(quote! { #name });
        } else {
            let d = &data_idents[bi];
            impl_args.push(quote! { &#d });
            bi += 1;
        }
    }

    quote! {
        #(#fn_attrs)*
        #fn_vis fn #impl_fn_name(#(#all_names: #all_types),*) #original_output #fn_block

        type #before_hook_name = unsafe extern "C" fn(#(*mut #biz_inners),*) -> bool;
        type #on_hook_name = unsafe extern "C" fn(#(*const #biz_inners),*) -> bool;
        type #after_hook_name = unsafe extern "C" fn(#(*const #biz_inners),*) -> bool;

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

        unsafe fn #run_before(#(#bp_idents: *mut #biz_inners),*) -> bool {
            if let Ok(list) = #before_hooks_static.lock() {
                for h in list.iter() {
                    unsafe {
                        if h(#(#bp_idents),*) {
                            return true;
                        }
                    }
                }
            }
            false
        }

        unsafe fn #run_on(#(#op_idents: *const #biz_inners),*) -> bool {
            if let Ok(list) = #on_hooks_static.lock() {
                for h in list.iter() {
                    unsafe {
                        if h(#(#op_idents),*) {
                            return true;
                        }
                    }
                }
            }
            false
        }

        unsafe fn #run_after(#(#ap_idents: *const #biz_inners),*) -> bool {
            if let Ok(list) = #after_hooks_static.lock() {
                for h in list.iter() {
                    unsafe {
                        if h(#(#ap_idents),*) {
                            return true;
                        }
                    }
                }
            }
            false
        }

        #[ffi_export]
        pub unsafe fn #wrapper_fn_name(#(#all_names: #all_types),*) #original_output {
            let reenter = #in_flag_static.load(Ordering::SeqCst);
            if reenter {
                return #impl_fn_name(#(#all_names),*);
            }
            #in_flag_static.store(true, Ordering::SeqCst);

            // 为每个业务参数克隆一份数据，BeforeHook 可以修改这些克隆；出参不克隆。
            #( let mut #data_idents = (#biz_names).clone(); )*
            #( let #bp_idents: *mut #biz_inners = &mut #data_idents; )*

            let mut interrupted = unsafe { #run_before(#(#bp_idents),*) };

            if !interrupted {
                #( let #op_idents: *const #biz_inners = &#data_idents; )*
                interrupted = unsafe { #run_on(#(#op_idents),*) };
            }

            let result = if !interrupted {
                #( let #ap_idents: *const #biz_inners = &#data_idents; )*
                // 原始函数接收修改后的业务克隆与透传的出参。
                let res = #impl_fn_name(#(#impl_args),*);
                unsafe { #run_after(#(#ap_idents),*) };
                res
            } else {
                #default_result
            };

            #in_flag_static.store(false, Ordering::SeqCst);
            result
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
