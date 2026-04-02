use bootloader::BiosBoot;
use std::{path::PathBuf, process::Command};

fn main() {
    let mut args = std::env::args().skip(1);

    let kernel = PathBuf::from(
        args.next()
            .expect("Usage: runner <kernel-elf> [--run]"),
    );

    let run_qemu = args.any(|a| a == "--run");

    // Place the disk image next to the kernel ELF
    let out_dir = kernel.parent().unwrap_or_else(|| std::path::Path::new("."));
    let disk_image = out_dir.join("kernel-bios.img");

    BiosBoot::new(&kernel)
        .create_disk_image(&disk_image)
        .expect("failed to create BIOS disk image");

    println!("disk_image:{}", disk_image.display());

    if run_qemu {
        let status = Command::new("qemu-system-x86_64")
            .arg("-drive")
            .arg(format!("format=raw,file={}", disk_image.display()))
            .arg("-m")
            .arg("512M")
            .arg("-serial")
            .arg("stdio")
            .arg("-no-reboot")
            .arg("-no-shutdown")
            .status()
            .expect("failed to launch qemu-system-x86_64 — is QEMU installed?");

        std::process::exit(status.code().unwrap_or(1));
    }
}
