pub fn run() {
    let mut any = false;
    crate::wasm::engine::for_each_instance(|handle, name, mem_pages| {
        crate::println!("[{}] {}  ({} page(s), {} KiB)", handle, name, mem_pages, mem_pages * 64);
        any = true;
    });
    if !any {
        crate::println!("(no instances)");
    }
}
