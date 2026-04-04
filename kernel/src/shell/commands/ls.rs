pub fn run() {
    let mut found = false;
    crate::fs::for_each_file(|name| {
        crate::println!("{}", name);
        found = true;
    });
    if !found {
        crate::println!("(no files registered)");
    }
}
