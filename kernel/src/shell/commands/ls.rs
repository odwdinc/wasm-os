pub fn run() {
    let mut found = false;
    crate::fs::for_each_file(|name, size| {
        crate::println!("  {:<24} {:>8} bytes", name, size);
        found = true;
    });
    if !found {
        crate::println!("(no files)");
    }
}
