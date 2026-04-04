pub fn run(args: &[&str]) {
    let mut first = true;
    for word in args {
        if !first { crate::print!(" "); }
        crate::print!("{}", word);
        first = false;
    }
    crate::println!();
}
