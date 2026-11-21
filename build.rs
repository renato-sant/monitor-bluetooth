//! Embute assets/monitorbt.ico no .exe.
//!
//! O toolchain GNU do rustup não traz `windres`, então este script monta à mão
//! um objeto COFF com a seção `.rsrc` (ícone + manifesto), igual ao que o
//! windres geraria, e o passa ao linker.

use std::path::PathBuf;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const RT_MANIFEST: u32 = 24;
const LANG: u32 = 0x0409;

fn main() {
    let ico_path = "assets/monitorbt.ico";
    let manifest_path = "assets/monitorbt.manifest";
    println!("cargo:rerun-if-changed={ico_path}");
    println!("cargo:rerun-if-changed={manifest_path}");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = std::fs::read(ico_path).expect("assets/monitorbt.ico");
    let (images, group) = parse_ico(&ico);
    let manifest = std::fs::read(manifest_path).expect("assets/monitorbt.manifest");
    let obj = build_coff(&images, &group, &manifest);

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("monitorbt-res.o");
    std::fs::write(&out, obj).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// Retorna as imagens do .ico e o diretório RT_GROUP_ICON correspondente.
fn parse_ico(ico: &[u8]) -> (Vec<Vec<u8>>, Vec<u8>) {
    let u16_at = |o: usize| u16::from_le_bytes([ico[o], ico[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes(ico[o..o + 4].try_into().unwrap());
    assert_eq!(u16_at(2), 1, "não é um .ico");
    let count = u16_at(4) as usize;

    let mut images = Vec::new();
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let e = 6 + i * 16;
        let size = u32_at(e + 8) as usize;
        let offset = u32_at(e + 12) as usize;
        images.push(ico[offset..offset + size].to_vec());
        // GRPICONDIRENTRY: os 12 primeiros bytes iguais ao ICONDIRENTRY + id (u16).
        group.extend_from_slice(&ico[e..e + 12]);
        group.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }
    (images, group)
}

fn align(buf: &mut Vec<u8>, n: usize) {
    while buf.len() % n != 0 {
        buf.push(0);
    }
}

fn dir_header(buf: &mut Vec<u8>, n_ids: u16) {
    buf.extend_from_slice(&[0u8; 12]); // characteristics, timestamp, versão
    buf.extend_from_slice(&0u16.to_le_bytes()); // entradas nomeadas
    buf.extend_from_slice(&n_ids.to_le_bytes()); // entradas por id
}

fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Monta a seção .rsrc e embrulha num objeto COFF x86-64.
fn build_coff(images: &[Vec<u8>], group: &[u8], manifest: &[u8]) -> Vec<u8> {
    // (tipo, id, dados), em ordem crescente de tipo e id.
    let mut res: Vec<(u32, u32, &[u8])> = images
        .iter()
        .enumerate()
        .map(|(i, d)| (RT_ICON, i as u32 + 1, d.as_slice()))
        .collect();
    res.push((RT_GROUP_ICON, 1, group));
    res.push((RT_MANIFEST, 1, manifest));
    let types = [RT_ICON, RT_GROUP_ICON, RT_MANIFEST];

    let mut s: Vec<u8> = Vec::new();
    // Nível 1: tipos.
    dir_header(&mut s, types.len() as u16);
    let type_entries = s.len();
    s.resize(s.len() + 8 * types.len(), 0);

    // Nível 2: ids de cada tipo; nível 3: idioma.
    let mut lang_slots = Vec::new(); // (posição da entrada de idioma, índice em res)
    for (ti, &t) in types.iter().enumerate() {
        let items: Vec<usize> = (0..res.len()).filter(|&i| res[i].0 == t).collect();
        let dir_off = s.len() as u32;
        put_u32(&mut s, type_entries + ti * 8, t);
        put_u32(&mut s, type_entries + ti * 8 + 4, 0x8000_0000 | dir_off);
        dir_header(&mut s, items.len() as u16);
        let id_entries = s.len();
        s.resize(s.len() + 8 * items.len(), 0);
        for (k, &ri) in items.iter().enumerate() {
            let lang_dir = s.len() as u32;
            put_u32(&mut s, id_entries + k * 8, res[ri].1);
            put_u32(&mut s, id_entries + k * 8 + 4, 0x8000_0000 | lang_dir);
            dir_header(&mut s, 1);
            lang_slots.push((s.len(), ri));
            s.resize(s.len() + 8, 0);
        }
    }

    // Data entries (16 bytes cada).
    let mut data_entries = Vec::new(); // (posição da data entry, índice em res)
    for &(slot, ri) in &lang_slots {
        let de = s.len();
        put_u32(&mut s, slot, LANG);
        put_u32(&mut s, slot + 4, de as u32);
        s.resize(s.len() + 16, 0);
        data_entries.push((de, ri));
    }

    // Dados; OffsetToData é RVA → relocação ADDR32NB contra o símbolo .rsrc.
    let mut relocs = Vec::new();
    for &(de, ri) in &data_entries {
        align(&mut s, 8);
        let data_off = s.len();
        s.extend_from_slice(res[ri].2);
        put_u32(&mut s, de, data_off as u32);
        put_u32(&mut s, de + 4, res[ri].2.len() as u32);
        relocs.push(de as u32);
    }
    align(&mut s, 8);

    // ---- Objeto COFF ----
    const HDR: usize = 20;
    const SEC: usize = 40;
    let raw_ptr = (HDR + SEC) as u32;
    let reloc_ptr = raw_ptr + s.len() as u32;
    let sym_ptr = reloc_ptr + relocs.len() as u32 * 10;

    let mut o = Vec::new();
    o.extend_from_slice(&0x8664u16.to_le_bytes()); // AMD64
    o.extend_from_slice(&1u16.to_le_bytes()); // 1 seção
    o.extend_from_slice(&0u32.to_le_bytes()); // timestamp
    o.extend_from_slice(&sym_ptr.to_le_bytes());
    o.extend_from_slice(&2u32.to_le_bytes()); // símbolo + aux
    o.extend_from_slice(&0u16.to_le_bytes()); // optional header
    o.extend_from_slice(&0u16.to_le_bytes()); // characteristics

    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes()); // VirtualSize
    o.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
    o.extend_from_slice(&(s.len() as u32).to_le_bytes());
    o.extend_from_slice(&raw_ptr.to_le_bytes());
    o.extend_from_slice(&reloc_ptr.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes()); // linenumbers
    o.extend_from_slice(&(relocs.len() as u16).to_le_bytes());
    o.extend_from_slice(&0u16.to_le_bytes());
    // INITIALIZED_DATA | ALIGN_8BYTES | MEM_READ
    o.extend_from_slice(&(0x0000_0040u32 | 0x0040_0000 | 0x4000_0000).to_le_bytes());

    o.extend_from_slice(&s);
    for r in &relocs {
        o.extend_from_slice(&r.to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes()); // símbolo 0 (.rsrc)
        o.extend_from_slice(&3u16.to_le_bytes()); // IMAGE_REL_AMD64_ADDR32NB
    }

    // Símbolo da seção + registro auxiliar.
    o.extend_from_slice(b".rsrc\0\0\0");
    o.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&1i16.to_le_bytes()); // seção 1
    o.extend_from_slice(&0u16.to_le_bytes());
    o.push(3); // IMAGE_SYM_CLASS_STATIC
    o.push(1); // 1 aux
    o.extend_from_slice(&(s.len() as u32).to_le_bytes());
    o.extend_from_slice(&(relocs.len() as u16).to_le_bytes());
    o.extend_from_slice(&[0u8; 12]);

    o.extend_from_slice(&4u32.to_le_bytes()); // string table vazia
    o
}
