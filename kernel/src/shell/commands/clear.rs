pub fn run() {
    // crate::vga::clear_screen();
    crate::print!("{}{}", crate::vga::CLEAR_SCREEN, crate::vga::CURSOR_POSITION);
}
