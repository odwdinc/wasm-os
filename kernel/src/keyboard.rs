use crate::vga::VgaBuffer;

pub fn read_loop(vga: &mut VgaBuffer) -> ! {
    loop {
        // placeholder: echo dummy input every second
        vga.write_str("You pressed a key!\n> ");
        for _ in 0..10000000 { core::hint::spin_loop(); }
    }
}