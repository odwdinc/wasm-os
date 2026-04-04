// save — flush in-memory file table to the Ramdisk (Sprint D.5)
//
// Serializes every registered file into the Ramdisk in WasmFS format.
// The Ramdisk is a static byte array; its contents are lost on cold reboot.
// True cross-reboot persistence requires a virtio-blk driver (Sprint D stretch).

pub fn run() {
    let n = crate::fs::save_to_ramdisk();
    crate::println!("saved {} file(s) to ramdisk", n);
}
