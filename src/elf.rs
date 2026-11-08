// SPDX-License-Identifier: GPL-3.0-or-later
//! Just enough ELF reading for AppImages: the AppImage type, architecture,
//! where the embedded filesystem starts, and the `.upd_info` section.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf {
    /// 0 = not an AppImage, 1 = ISO 9660 (type 1), 2 = SquashFS/DwarFS (type 2).
    pub appimage_type: u8,
    pub arch: &'static str,
    /// End of the ELF runtime: the embedded filesystem image starts here.
    pub payload_offset: u64,
    pub sections: Vec<(String, u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    SquashFs,
    DwarFs,
    Unknown,
}

fn read_at(file: &mut File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0; len];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn arch_name(machine: u16) -> &'static str {
    match machine {
        0x3e => "x86_64",
        0xb7 => "aarch64",
        0x03 => "i386",
        0x28 => "armhf",
        0xf3 => "riscv64",
        _ => "unknown",
    }
}

/// The architecture this program was built for, named like `arch_name`.
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i386",
        "arm" => "armhf",
        "riscv64" => "riscv64",
        _ => "unknown",
    }
}

pub fn read(path: &Path) -> Result<Elf, String> {
    let mut file = File::open(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let ident = read_at(&mut file, 0, 64).map_err(|_| "File is too small to be an AppImage")?;
    if &ident[0..4] != b"\x7fELF" {
        return Err("Not an ELF executable, so not an AppImage".into());
    }
    let wide = match ident[4] {
        1 => false,
        2 => true,
        _ => return Err("Unknown ELF class".into()),
    };
    let le = match ident[5] {
        1 => true,
        2 => false,
        _ => return Err("Unknown ELF byte order".into()),
    };
    let u16_at = |b: &[u8], o: usize| {
        let v = [b[o], b[o + 1]];
        if le {
            u16::from_le_bytes(v)
        } else {
            u16::from_be_bytes(v)
        }
    };
    let u32_at = |b: &[u8], o: usize| {
        let v = [b[o], b[o + 1], b[o + 2], b[o + 3]];
        if le {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }
    };
    let u64_at = |b: &[u8], o: usize| {
        let mut v = [0; 8];
        v.copy_from_slice(&b[o..o + 8]);
        if le {
            u64::from_le_bytes(v)
        } else {
            u64::from_be_bytes(v)
        }
    };
    let appimage_type = match &ident[8..11] {
        b"AI\x01" => 1,
        b"AI\x02" => 2,
        _ => 0,
    };
    let arch = arch_name(u16_at(&ident, 18));
    let (shoff, shentsize, shnum, shstrndx) = if wide {
        (
            u64_at(&ident, 0x28),
            u16_at(&ident, 0x3a) as u64,
            u16_at(&ident, 0x3c) as u64,
            u16_at(&ident, 0x3e) as u64,
        )
    } else {
        (
            u32_at(&ident, 0x20) as u64,
            u16_at(&ident, 0x2e) as u64,
            u16_at(&ident, 0x30) as u64,
            u16_at(&ident, 0x32) as u64,
        )
    };
    let payload_offset = shoff + shentsize * shnum;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if shnum > 4096 || shentsize > 256 || payload_offset > len {
        return Err("Damaged ELF section table".into());
    }
    let table = read_at(&mut file, shoff, (shentsize * shnum) as usize)
        .map_err(|_| "Cannot read the ELF section table")?;
    let header = |i: u64| -> (u32, u64, u64) {
        let b = &table[(i * shentsize) as usize..];
        if wide {
            (u32_at(b, 0), u64_at(b, 24), u64_at(b, 32))
        } else {
            (u32_at(b, 0), u32_at(b, 16) as u64, u32_at(b, 20) as u64)
        }
    };
    let mut sections = Vec::new();
    if shstrndx < shnum {
        let (_, stroff, strsize) = header(shstrndx);
        if strsize < 1 << 20
            && let Ok(names) = read_at(&mut file, stroff, strsize as usize)
        {
            for i in 0..shnum {
                let (name, offset, size) = header(i);
                let name = names
                    .get(name as usize..)
                    .and_then(|s| s.split(|b| *b == 0).next())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .unwrap_or_default();
                sections.push((name, offset, size));
            }
        }
    }
    Ok(Elf {
        appimage_type,
        arch,
        payload_offset,
        sections,
    })
}

impl Elf {
    /// A section's contents as text up to its first NUL, trimmed.
    pub fn section_text(&self, path: &Path, name: &str) -> Option<String> {
        let (_, offset, size) = self.sections.iter().find(|(n, ..)| n == name)?;
        if *size == 0 || *size > 1 << 16 {
            return None;
        }
        let mut file = File::open(path).ok()?;
        let bytes = read_at(&mut file, *offset, *size as usize).ok()?;
        let text = bytes.split(|b| *b == 0).next().unwrap_or_default();
        let text = String::from_utf8_lossy(text).trim().to_string();
        (!text.is_empty()).then_some(text)
    }
    pub fn payload(&self, path: &Path) -> Payload {
        let Ok(mut file) = File::open(path) else {
            return Payload::Unknown;
        };
        match read_at(&mut file, self.payload_offset, 6) {
            Ok(m) if &m[..4] == b"hsqs" => Payload::SquashFs,
            Ok(m) if &m[..6] == b"DWARFS" => Payload::DwarFs,
            _ => Payload::Unknown,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    /// A minimal little-endian ELF64 with a section-name table and a
    /// `.upd_info` section, followed by `payload`.
    pub fn tiny_elf(update_info: &str, payload: &[u8]) -> Vec<u8> {
        let names = b"\0.shstrtab\0.upd_info\0";
        let info = update_info.as_bytes();
        let mut out = vec![0u8; 64];
        out[0..4].copy_from_slice(b"\x7fELF");
        out[4] = 2;
        out[5] = 1;
        out[6] = 1;
        out[8..11].copy_from_slice(b"AI\x02");
        out[16..18].copy_from_slice(&2u16.to_le_bytes());
        out[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
        let names_off = 64u64;
        let info_off = names_off + names.len() as u64;
        let shoff = info_off + info.len() as u64;
        out.extend_from_slice(names);
        out.extend_from_slice(info);
        out[0x28..0x30].copy_from_slice(&shoff.to_le_bytes());
        out[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        out[0x3c..0x3e].copy_from_slice(&3u16.to_le_bytes());
        out[0x3e..0x40].copy_from_slice(&1u16.to_le_bytes());
        let section = |name: u32, off: u64, size: u64| {
            let mut s = vec![0u8; 64];
            s[0..4].copy_from_slice(&name.to_le_bytes());
            s[24..32].copy_from_slice(&off.to_le_bytes());
            s[32..40].copy_from_slice(&size.to_le_bytes());
            s
        };
        out.extend(section(0, 0, 0));
        out.extend(section(1, names_off, names.len() as u64));
        out.extend(section(11, info_off, info.len() as u64));
        out.extend_from_slice(payload);
        out
    }
    #[test]
    fn reads_type_arch_offset_and_update_info() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.AppImage");
        let bytes = tiny_elf("gh-releases-zsync|o|r|latest|A-*.zsync", b"hsqs....");
        std::fs::write(&path, &bytes).unwrap();
        let elf = read(&path).unwrap();
        assert_eq!(elf.appimage_type, 2);
        assert_eq!(elf.arch, "x86_64");
        assert_eq!(elf.payload_offset as usize, bytes.len() - 8);
        assert_eq!(elf.payload(&path), Payload::SquashFs);
        assert_eq!(
            elf.section_text(&path, ".upd_info").as_deref(),
            Some("gh-releases-zsync|o|r|latest|A-*.zsync")
        );
        std::fs::write(&path, b"#!/bin/sh\n").unwrap();
        assert!(read(&path).is_err());
    }
}
