pub fn run(name: &str) {
    if name.is_empty() {
        let t = crate::drivers::pit::ticks();
        crate::println!("ticks: {}  (~{} s)", t, t / 100);
        return;
    }
    let data = match crate::fs::find_file(name) {
        Some(d) => d,
        None    => { crate::println!("not found: {}", name); return; }
    };
    let module = match crate::wasm::loader::load(data) {
        Ok(m)  => m,
        Err(e) => { crate::println!("load error: {}", e.as_str()); return; }
    };
    let func_count = module.function_section
        .and_then(|s| crate::wasm::loader::read_u32_leb128(s))
        .map(|(n, _)| n)
        .unwrap_or(0);
    let import_count = crate::wasm::engine::count_func_imports(module.import_section);
    let export_count = module.export_section
        .and_then(|s| crate::wasm::loader::read_u32_leb128(s))
        .map(|(n, _)| n)
        .unwrap_or(0);
    crate::println!("file:    {}", name);
    crate::println!("funcs:   {} defined, {} imported", func_count, import_count);
    crate::println!("exports: {}", export_count);
}
