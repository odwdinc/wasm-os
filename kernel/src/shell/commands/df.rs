// df — report filesystem space usage

pub fn run() {
    match crate::fs::fat::fat_disk_stats() {
        None => {
            crate::println!("df: filesystem not mounted");
        }
        Some((total, free)) => {
            let used = total.saturating_sub(free);
            crate::println!("Filesystem    Size (K)  Used (K)  Avail (K)");
            crate::println!("FAT          {:>8}  {:>8}  {:>8}",
                total / 1024, used / 1024, free / 1024);
        }
    }
}
