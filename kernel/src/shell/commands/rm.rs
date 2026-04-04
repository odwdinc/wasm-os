pub fn run(argv: &[&str]) {
    if argv.is_empty() {
        crate::println!("usage: rm <name>");
        return;
    }
    let name = argv[0];
    if crate::fs::remove_file(name) {
        crate::println!("removed {}", name);
    } else {
        crate::println!("rm: {}: not found", name);
    }
}
