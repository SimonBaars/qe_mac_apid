#[allow(non_upper_case_globals)]
#[allow(non_snake_case)]
#[allow(unused)]
#[allow(non_camel_case_types)]
#[allow(improper_ctypes)]
mod modelinfo;

mod io_subset;
mod oui;
mod plist_data;
mod qcow2;
mod serial;

use std::{
    io::{stdin, stdout, Seek, SeekFrom, Write},
    path::PathBuf,
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use fatfs::FileSystem;
use gpt::{disk::LogicalBlockSize, GptConfig};
use io_subset::IoSubset;
use oui::APPLE_OUIS;
use plist_data::MacPlist;
use qcow2::Qcow2;
use rand::seq::IndexedRandom;
use uuid::Uuid;

const HEX_DIGITS: &[u8] = b"01234556789abcdef";

fn main() -> Result<()> {
    let args = Args::parse();
    env_logger::builder()
        .filter_level(args.verbose.log_level_filter())
        .init();

    let mut qcow2 = Qcow2::new(&args.bootloader, args.dry_run)?;
    let mut first_partition = first_partition_subset(&mut qcow2)?;

    let fs = FileSystem::new(&mut first_partition, fatfs::FsOptions::new())
        .context("Failed to open FAT32 filesystem")?;

    let mut conf_plist = fs
        .root_dir()
        .open_file("EFI/OC/config.plist")
        .context("Failed to open config.plist")?;

    let mut plist: MacPlist = plist::from_reader(&mut conf_plist)?;

    let mut needs_update = false;

    // Check if valid serials already exist
    if plist.has_valid_serials() && !args.force_regenerate {
        println!("Valid serial numbers already configured:");
        println!("  Serial Number: {}", plist.get_serial_number());
        println!("  MLB: {}", plist.get_mlb());
        println!();
        
        if !args.dry_run {
            print!("Do you want to regenerate new serial numbers? (y/N) ");
            stdout().flush()?;
            let mut buffer = String::new();
            stdin().read_line(&mut buffer)?;
            if !buffer.trim().eq_ignore_ascii_case("y") && !buffer.trim().eq_ignore_ascii_case("yes") {
                println!("Keeping existing serial numbers.");
            } else {
                needs_update = true;
            }
        }
    } else {
        needs_update = true;
    }

    let serial = if needs_update {
        serial::find_desired(plist.get_product_name())?
    } else {
        serial::Serial {
            serial_number: plist.get_serial_number().to_string(),
            board_serial: plist.get_mlb().to_string(),
        }
    };

    let uuid = if needs_update {
        Uuid::new_v4()
    } else {
        // Keep existing UUID
        Uuid::new_v4() // We'll keep this for now; ideally we'd parse the existing one
    };

    let rom = if needs_update {
        let mut rom = [0; 12];
        let mut rng = rand::rng();

        let rom_start = APPLE_OUIS
            .choose(&mut rng)
            .context("Couldn't find an Apple OUI")?;
        if rom_start.len() != 6 {
            bail!("Rom start length should be 6 bytes");
        }
        rom[..6].copy_from_slice(rom_start.as_bytes());

        for rom_byte in rom[6..].iter_mut() {
            *rom_byte = *HEX_DIGITS
                .choose(&mut rng)
                .context("Hex digits couldn't be generated")?;
        }

        rom
    } else {
        [0; 12] // Keep existing ROM
    };

    // Check and add Sequoia patches if requested
    let mut patches_added = false;
    if args.add_sequoia_patches || args.force_sequoia_patches {
        if !plist.has_sequoia_patches() || args.force_sequoia_patches {
            println!();
            println!("Adding macOS Sequoia kernel patches for VM detection bypass...");
            plist.add_sequoia_kernel_patches();
            patches_added = true;
        } else {
            println!();
            println!("Sequoia kernel patches already present.");
        }
    } else if !plist.has_sequoia_patches() {
        println!();
        println!("Note: Sequoia kernel patches not found in config.plist.");
        println!("For macOS Sequoia 15.7.1+, you need kernel patches to enable Apple ID login.");
        print!("Would you like to add them now? (Y/n) ");
        stdout().flush()?;
        let mut buffer = String::new();
        stdin().read_line(&mut buffer)?;
        let answer = buffer.trim();
        if answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
            plist.add_sequoia_kernel_patches();
            patches_added = true;
        }
    }

    if args.dry_run {
        println!();
        if needs_update {
            println!("Would set serial number to {}", serial.serial_number);
            println!("Would set MLB to {}", serial.board_serial);
            println!("Would set UUID to {}", uuid);
            println!(
                "Would set ROM to {:?}",
                std::str::from_utf8(&rom).context("ROM should always be valid UTF-8")?
            );
        }
        if patches_added {
            println!("Would add Sequoia kernel patches");
        }
        return Ok(());
    }

    // Only update if changes were made
    if needs_update || patches_added {
        if needs_update {
            plist.set_serial_number(serial.serial_number);
            plist.set_mlb(serial.board_serial);
            plist.set_uuid(uuid);
            plist.set_rom(rom);
        }

        plist.debug();
        conf_plist
            .truncate()
            .context("Failed to truncate config.plist")?;
        conf_plist.seek(SeekFrom::Start(0))?;
        plist::to_writer_xml(&mut conf_plist, &plist).context("Failed to write config.plist")?;

        conf_plist.flush()?;
        drop(conf_plist);
        fs.unmount()?;
        first_partition.flush()?;
        qcow2.flush()?;

        println!();
        println!("✓ Configuration updated successfully!");
        if patches_added {
            println!("✓ Sequoia kernel patches added");
            println!();
            println!("IMPORTANT: These patches may prevent macOS system updates.");
            println!("To update macOS:");
            println!("  1. Temporarily disable the patches in OpenCore boot menu");
            println!("  2. Reset NVRAM");
            println!("  3. Install the update");
            println!("  4. Re-enable the patches after updating");
        }
    } else {
        println!();
        println!("No changes needed.");
    }

    Ok(())
}

fn first_partition_subset(mut qcow2: &mut Qcow2) -> Result<IoSubset<&mut Qcow2>> {
    let disk = GptConfig::new().open_from_device(&mut qcow2)?;

    let partitions = disk.partitions();
    let partition = partitions.get(&1).context("Failed to get partition")?;

    let start = partition.bytes_start(LogicalBlockSize::Lb512)?;
    let end = start + partition.bytes_len(LogicalBlockSize::Lb512)?;

    Ok(IoSubset::new(qcow2, start, end))
}

#[derive(Parser)]
struct Args {
    #[clap(long, help = "Path to the bootloader ('OpenCore.qcow2')")]
    bootloader: PathBuf,
    
    #[clap(short, long, help = "Don't commit changes to disk")]
    dry_run: bool,
    
    #[clap(short = 'f', long, help = "Force regeneration of serial numbers even if valid ones exist")]
    force_regenerate: bool,
    
    #[clap(short = 's', long, help = "Add Sequoia kernel patches for VM detection bypass")]
    add_sequoia_patches: bool,
    
    #[clap(long, help = "Force add Sequoia patches even if they already exist")]
    force_sequoia_patches: bool,
    
    #[clap(flatten)]
    verbose: clap_verbosity_flag::Verbosity<clap_verbosity_flag::WarnLevel>,
}
