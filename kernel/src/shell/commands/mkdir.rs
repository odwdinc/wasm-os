// mkdir <dir> — create a directory on the FAT volume

pub fn run(argv: &[&str]) {
    if argv.is_empty() {
        crate::println!("usage: mkdir <dir>");
        return;
    }
    let name = argv[0];
    if !crate::fs::fat::fat_mkdir(name) {
        crate::println!("mkdir: {}: cannot create directory", name);
    }
}
