pub fn run() {
    let len = crate::shell::history_len();
    for i in 0..len {
        crate::println!("{:>4}  {}", i + 1, crate::shell::history_get(i));
    }
}
