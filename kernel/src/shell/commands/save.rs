// save — flush in-memory file table to the FAT disk volume
//
// Files added via `write` during a session are in-memory only.
// `save` writes them to the mounted FAT filesystem so they persist across
// reboots (virtio-blk) or remain available in-session (ramdisk fallback).

pub fn run() {
    let n = crate::fs::save_to_fat();
    crate::println!("saved {} file(s) to FAT volume", n);
}
