// ls — list files and directories in the current working directory

pub fn run() {
    let cwd = crate::shell::get_cwd();
    let mut found = false;

    crate::fs::fat::fat_list_path(cwd, |name, size, is_dir| {
        if is_dir {
            crate::println!("  {:<24} <DIR>", name);
        } else {
            crate::println!("  {:<24} {:>8} bytes", name, size);
        }
        found = true;
    });

    if !found {
        crate::println!("(no files)");
    }
}
