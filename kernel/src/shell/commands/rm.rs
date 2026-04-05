pub fn run(argv: &[&str]) {
    if argv.is_empty() {
        crate::println!("usage: rm <name>");
        return;
    }
    let name = argv[0];
    // Remove from the FAT volume (persists across reboots).
    let fat_ok = crate::fs::fat::fat_remove_file(name);
    // Remove from the in-memory table too.
    let mem_ok = crate::fs::remove_file(name);
    if fat_ok || mem_ok {
        crate::println!("removed {}", name);
    } else {
        crate::println!("rm: {}: not found", name);
    }
}
