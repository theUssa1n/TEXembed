use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use texembed::{
    load_streamtex_file, load_textures_file, save_streamtex_file, save_streamtex_with_sidecar,
    save_textures_file,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("texembed-preservation-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn dds(width: u32, height: u32, value: u8) -> Vec<u8> {
    let length = (width.div_ceil(4) * height.div_ceil(4) * 8) as usize;
    let mut bytes = vec![0; 128 + length];
    bytes[..4].copy_from_slice(b"DDS ");
    put_u32(&mut bytes, 4, 124);
    put_u32(&mut bytes, 8, 0x81007);
    put_u32(&mut bytes, 12, height);
    put_u32(&mut bytes, 16, width);
    put_u32(&mut bytes, 20, length as u32);
    put_u32(&mut bytes, 28, 1);
    put_u32(&mut bytes, 76, 32);
    put_u32(&mut bytes, 80, 4);
    bytes[84..88].copy_from_slice(b"DXT1");
    put_u32(&mut bytes, 108, 0x1000);
    bytes[128..].fill(value);
    bytes
}

fn stream(records: &[Vec<u8>], tail: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for record in records {
        bytes.extend_from_slice(&(record.len() as u32).to_le_bytes());
        bytes.extend_from_slice(record);
    }
    bytes.extend_from_slice(tail);
    bytes
}

struct Catalog {
    data: Vec<u8>,
    offsets: Vec<u32>,
}

impl Catalog {
    fn embedded(records: &[Vec<u8>]) -> Self {
        let mut data = vec![0xAB; 7];
        let mut offsets = Vec::new();
        for record in records {
            offsets.push(data.len() as u32);
            data.extend_from_slice(&(record.len() as u32).to_le_bytes());
            data.extend_from_slice(record);
            data.extend_from_slice(&[0xCD; 5]);
        }
        data.extend_from_slice(&[0xEF; 3]);
        Self { data, offsets }
    }

    fn streamed() -> Self {
        Self {
            data: Vec::new(),
            offsets: vec![0, 0x1234_5678],
        }
    }
}

struct TexturesFixture {
    bytes: Vec<u8>,
    catalogs: Vec<usize>,
    entries: Vec<Vec<usize>>,
    records: Vec<Vec<usize>>,
    data: Vec<usize>,
}

fn textures(catalogs: &[Catalog]) -> TexturesFixture {
    let count: usize = catalogs.iter().map(|catalog| catalog.offsets.len()).sum();
    let entries_start = 40 + catalogs.len() * 16;
    let records_start = entries_start + count * 12;
    let header_end = records_start + count * 64;
    let mut bytes = vec![0; header_end];
    bytes[..4].copy_from_slice(b"SXET");
    put_u32(&mut bytes, 4, 12);
    put_u32(&mut bytes, 12, (header_end - 16) as u32);
    put_u32(&mut bytes, 16, count as u32);
    put_u32(&mut bytes, 20, (records_start - 16) as u32);
    put_u32(&mut bytes, 32, catalogs.len() as u32);
    put_u32(&mut bytes, 36, 24);
    let mut fixture = TexturesFixture {
        bytes,
        catalogs: Vec::new(),
        entries: Vec::new(),
        records: Vec::new(),
        data: Vec::new(),
    };
    let mut global = 0;
    for (index, catalog) in catalogs.iter().enumerate() {
        let cat = 40 + index * 16;
        fixture.catalogs.push(cat);
        put_u32(&mut fixture.bytes, cat, catalog.data.len() as u32);
        put_u32(&mut fixture.bytes, cat + 4, 0xFEDC_BA98);
        put_u32(&mut fixture.bytes, cat + 8, catalog.offsets.len() as u32);
        put_u32(
            &mut fixture.bytes,
            cat + 12,
            (entries_start + global * 12 - 16) as u32,
        );
        let mut entries = Vec::new();
        let mut records = Vec::new();
        for offset in &catalog.offsets {
            let entry = entries_start + global * 12;
            let record = records_start + global * 64;
            entries.push(entry);
            records.push(record);
            fixture.bytes[record..record + 64].fill(0x73);
            put_u32(&mut fixture.bytes, record + 4, global as u32 + 1);
            put_u32(&mut fixture.bytes, entry, (record - 16) as u32);
            put_u32(&mut fixture.bytes, entry + 4, 0xAABB_CCDD);
            put_u32(&mut fixture.bytes, entry + 8, *offset);
            global += 1;
        }
        fixture.entries.push(entries);
        fixture.records.push(records);
        fixture.data.push(fixture.bytes.len());
        fixture.bytes.extend_from_slice(&catalog.data);
    }
    fixture
        .bytes
        .extend_from_slice(b"MN2C\xFF\xFE\x00unknown tail\x00");
    fixture
}

struct PairedSidecar {
    bytes: Vec<u8>,
    entries: Vec<usize>,
    records: Vec<usize>,
}

/// Builds a streamed `.textures` sidecar whose entry `data_offset`s and record
/// header shapes reference real `.streamtex` record positions.
fn paired_sidecar(pairs: &[(u32, (u32, u32))]) -> PairedSidecar {
    let count = pairs.len();
    let entries_start = 56;
    let records_start = entries_start + count * 12;
    let header_end = records_start + count * 64;
    let mut bytes = vec![0; header_end];
    bytes[..4].copy_from_slice(b"SXET");
    put_u32(&mut bytes, 4, 12);
    put_u32(&mut bytes, 12, (header_end - 16) as u32);
    put_u32(&mut bytes, 16, count as u32);
    put_u32(&mut bytes, 20, (records_start - 16) as u32);
    put_u32(&mut bytes, 32, 1);
    put_u32(&mut bytes, 36, 24);
    put_u32(&mut bytes, 44, 0xFEDC_BA98);
    put_u32(&mut bytes, 48, count as u32);
    put_u32(&mut bytes, 52, 40);
    let mut entries = Vec::new();
    let mut records = Vec::new();
    for (index, &(data_offset, (width, height))) in pairs.iter().enumerate() {
        let entry = entries_start + index * 12;
        let record = records_start + index * 64;
        entries.push(entry);
        records.push(record);
        bytes[record..record + 64].fill(0x73);
        put_u32(&mut bytes, record + 4, index as u32 + 1);
        put_u32(&mut bytes, record + 12, 0x21);
        put_u32(&mut bytes, record + 16, 1);
        put_u32(&mut bytes, record + 20, 1);
        put_u32(&mut bytes, record + 24, width);
        put_u32(&mut bytes, record + 28, height);
        put_u32(&mut bytes, entry, (record - 16) as u32);
        put_u32(&mut bytes, entry + 4, 0xAABB_CCDD);
        put_u32(&mut bytes, entry + 8, data_offset);
    }
    bytes.extend_from_slice(b"MN2C\xFF\xFE\x00");
    PairedSidecar {
        bytes,
        entries,
        records,
    }
}

#[test]
fn open_and_save_without_changes_produces_identical_file() {
    let directory = TestDirectory::new();
    let input = directory.path("original.textures");
    let output = directory.path("saved.textures");
    let fixture = textures(&[Catalog::embedded(&[dds(8, 8, 1), dds(16, 4, 2)])]);
    fs::write(&input, &fixture.bytes).unwrap();
    let file = load_textures_file(input).unwrap();
    assert!(file.dds_list.iter().all(|dds| !dds.modified));
    save_textures_file(&file, &output).unwrap();
    assert_eq!(fs::read(output).unwrap(), fixture.bytes);
}

#[test]
fn mixed_textures_no_op_save_preserves_streamed_catalogs_in_either_order() {
    let directory = TestDirectory::new();
    for streamed_first in [false, true] {
        let embedded = Catalog::embedded(&[dds(8, 8, 1)]);
        let catalogs = if streamed_first {
            vec![Catalog::streamed(), embedded]
        } else {
            vec![embedded, Catalog::streamed()]
        };
        let fixture = textures(&catalogs);
        let path = directory.path("mixed.textures");
        fs::write(&path, &fixture.bytes).unwrap();
        let file = load_textures_file(path.clone()).unwrap();
        assert_eq!(file.dds_list.len(), 1);
        assert_eq!(file.dds_list[0].catalog_index, usize::from(streamed_first));
        save_textures_file(&file, &path).unwrap();
        assert_eq!(fs::read(path).unwrap(), fixture.bytes);
    }
}

#[test]
fn replacing_embedded_data_preserves_streamed_headers_and_untouched_records() {
    let directory = TestDirectory::new();
    let original_dds = dds(8, 8, 2);
    let fixture = textures(&[
        Catalog::streamed(),
        Catalog::embedded(&[dds(4, 4, 1), original_dds.clone()]),
        Catalog::streamed(),
    ]);
    let path = directory.path("mixed.textures");
    fs::write(&path, &fixture.bytes).unwrap();
    let mut file = load_textures_file(path.clone()).unwrap();
    let replacement = dds(32, 16, 9);
    file.dds_list[0].bytes = replacement.clone().into();
    file.dds_list[0].modified = true;
    save_textures_file(&file, &path).unwrap();
    let saved = fs::read(&path).unwrap();
    for index in [0, 2] {
        let cat = fixture.catalogs[index];
        assert_eq!(&saved[cat..cat + 16], &fixture.bytes[cat..cat + 16]);
        for entry in &fixture.entries[index] {
            assert_eq!(
                &saved[*entry..*entry + 12],
                &fixture.bytes[*entry..*entry + 12]
            );
        }
        for record in &fixture.records[index] {
            assert_eq!(
                &saved[*record..*record + 64],
                &fixture.bytes[*record..*record + 64]
            );
        }
    }
    let untouched = fixture.records[1][1];
    assert_eq!(
        &saved[untouched..untouched + 64],
        &fixture.bytes[untouched..untouched + 64]
    );
    let reloaded = load_textures_file(path).unwrap();
    assert_eq!(&*reloaded.dds_list[0].bytes, replacement);
    assert_eq!(&*reloaded.dds_list[1].bytes, original_dds);
    assert!(saved.ends_with(b"MN2C\xFF\xFE\x00unknown tail\x00"));
    assert!(saved.windows(7).any(|bytes| bytes == [0xAB; 7]));
    assert!(saved.windows(5).any(|bytes| bytes == [0xCD; 5]));
}

#[test]
fn streamtex_round_trip_preserves_opaque_records_and_trailing_bytes() {
    let directory = TestDirectory::new();
    let path = directory.path("opaque.streamtex");
    let records = [
        vec![0x51; 17],
        dds(8, 8, 2),
        vec![0xA4; 32],
        Vec::new(),
        dds(4, 4, 3),
    ];
    for tail in [
        &b"\xFC\xFD\xFE"[..],
        &b"\xFF\xFF\xFF\xFFopaque trailing data"[..],
    ] {
        let original = stream(&records, tail);
        fs::write(&path, &original).unwrap();
        let file = load_streamtex_file(path.clone()).unwrap();
        assert!(file.original_bytes.is_empty());
        assert!(file.streamtex_layout.is_some());
        assert_eq!(file.dds_list.len(), 2);
        assert_eq!(file.dds_list[0].entry_index, 1);
        assert_eq!(file.dds_list[1].entry_index, 4);
        save_streamtex_file(&file, &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}

#[test]
fn streamtex_replacement_preserves_unknown_records_and_other_prefixes() {
    let directory = TestDirectory::new();
    let path = directory.path("replacement.streamtex");
    let first = dds(8, 8, 1);
    let opaque = vec![0xAA; 31];
    let last = dds(16, 16, 3);
    let tail = b"\xFF\xFF\xFF\xFFtail";
    let original = stream(&[first, opaque.clone(), last.clone()], tail);
    fs::write(&path, original).unwrap();
    let mut file = load_streamtex_file(path.clone()).unwrap();
    let replacement = dds(32, 32, 4);
    file.dds_list[0].bytes = replacement.clone().into();
    file.dds_list[0].modified = true;
    save_streamtex_file(&file, &path).unwrap();
    assert_eq!(
        fs::read(path).unwrap(),
        stream(&[replacement, opaque, last], tail)
    );
}

#[test]
fn recovered_streamtex_length_prefix_is_not_normalized_on_no_op_save() {
    let directory = TestDirectory::new();
    let path = directory.path("stale.streamtex");
    let first = dds(8, 8, 1);
    let mut original = stream(&[first.clone(), dds(16, 16, 2)], b"");
    put_u32(&mut original, 0, first.len() as u32 - 4);
    fs::write(&path, &original).unwrap();
    let file = load_streamtex_file(path.clone()).unwrap();
    assert_eq!(file.dds_list.len(), 2);
    save_streamtex_file(&file, &path).unwrap();
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn textures_with_only_streamed_catalogs_round_trip() {
    let directory = TestDirectory::new();
    let path = directory.path("streamed.textures");
    let fixture = textures(&[Catalog::streamed(), Catalog::streamed()]);
    fs::write(&path, &fixture.bytes).unwrap();
    let file = load_textures_file(path.clone()).unwrap();
    assert!(file.dds_list.is_empty());
    save_textures_file(&file, &path).unwrap();
    assert_eq!(fs::read(path).unwrap(), fixture.bytes);
}

#[test]
fn malformed_catalog_offsets_return_errors_without_overwriting_destination() {
    let directory = TestDirectory::new();
    let input = directory.path("source.textures");
    let output = directory.path("saved.textures");
    let fixture = textures(&[Catalog::embedded(&[dds(8, 8, 2)])]);
    fs::write(&input, &fixture.bytes).unwrap();
    for offset in [
        12,
        32,
        36,
        fixture.catalogs[0],
        fixture.catalogs[0] + 8,
        fixture.catalogs[0] + 12,
        fixture.entries[0][0],
        fixture.entries[0][0] + 8,
        fixture.data[0] + 7,
    ] {
        let mut file = load_textures_file(input.clone()).unwrap();
        put_u32(&mut file.original_bytes, offset, u32::MAX);
        fs::write(&output, b"unchanged destination").unwrap();
        let result = std::panic::catch_unwind(|| save_textures_file(&file, &output));
        assert!(result.is_ok(), "Save panicked for offset {offset}");
        assert!(
            result.unwrap().is_err(),
            "Save accepted invalid offset {offset}"
        );
        assert_eq!(fs::read(&output).unwrap(), b"unchanged destination");
    }
}

#[test]
fn truncated_textures_never_panic() {
    let directory = TestDirectory::new();
    let path = directory.path("truncated.textures");
    let fixture = textures(&[Catalog::embedded(&[dds(8, 8, 2)])]);
    for length in 0..fixture.data[0] + 7 + 4 + 160 {
        fs::write(&path, &fixture.bytes[..length]).unwrap();
        let result = std::panic::catch_unwind(|| load_textures_file(path.clone()));
        assert!(result.is_ok(), "Load panicked at length {length}");
        assert!(
            result.unwrap().is_err(),
            "Load accepted truncated catalog at length {length}"
        );
    }
}

#[test]
fn malformed_streams_never_panic_or_replace_destination() {
    let directory = TestDirectory::new();
    let path = directory.path("malformed.streamtex");
    for data in [
        Vec::new(),
        vec![0; 7],
        b"TEXSinvalid".to_vec(),
        stream(&[vec![0x23; 33]], b""),
        u32::MAX.to_le_bytes().to_vec(),
    ] {
        fs::write(&path, data).unwrap();
        let result = std::panic::catch_unwind(|| load_streamtex_file(path.clone()));
        assert!(result.is_ok());
        assert!(result.unwrap().is_err());
    }
    fs::write(&path, stream(&[dds(8, 8, 1)], b"")).unwrap();
    let mut file = load_streamtex_file(path.clone()).unwrap();
    let original = fs::read(&path).unwrap();
    file.dds_list[0].entry_index = usize::MAX;
    assert!(save_streamtex_file(&file, &path).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn duplicated_payload_offsets_are_preserved_and_replacement_detaches_only_one_entry() {
    let directory = TestDirectory::new();
    let path = directory.path("aliases.textures");
    let original_dds = dds(8, 8, 2);
    let mut catalog = Catalog::embedded(std::slice::from_ref(&original_dds));
    catalog.offsets.push(catalog.offsets[0]);
    let fixture = textures(&[catalog]);
    fs::write(&path, &fixture.bytes).unwrap();
    let mut file = load_textures_file(path.clone()).unwrap();
    save_textures_file(&file, &path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), fixture.bytes);
    let replacement = dds(16, 16, 3);
    file.dds_list[0].bytes = replacement.clone().into();
    file.dds_list[0].modified = true;
    save_textures_file(&file, &path).unwrap();
    let loaded = load_textures_file(path.clone()).unwrap();
    assert_eq!(&*loaded.dds_list[0].bytes, replacement);
    assert_eq!(&*loaded.dds_list[1].bytes, original_dds);
    let saved = fs::read(path).unwrap();
    assert_eq!(get_u32(&saved, fixture.entries[0][1] + 8), 7);
    assert_ne!(get_u32(&saved, fixture.entries[0][0] + 8), 7);
}

#[test]
fn invalid_embedded_dds_is_preserved_during_replacement() {
    let directory = TestDirectory::new();
    let path = directory.path("opaque.textures");
    let opaque = vec![0x4F; 37];
    let fixture = textures(&[Catalog::embedded(&[
        opaque.clone(),
        dds(4, 4, 1),
        dds(8, 8, 2),
    ])]);
    fs::write(&path, &fixture.bytes).unwrap();
    let mut file = load_textures_file(path.clone()).unwrap();
    assert_eq!(file.dds_list.len(), 2);
    save_textures_file(&file, &path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), fixture.bytes);
    file.dds_list[1].bytes = dds(16, 16, 7).into();
    file.dds_list[1].modified = true;
    save_textures_file(&file, &path).unwrap();
    let saved = fs::read(path).unwrap();
    let start = fixture.data[0] + 7;
    assert_eq!(&saved[start + 4..start + 4 + opaque.len()], opaque);
    assert_eq!(
        &saved[fixture.records[0][0]..fixture.records[0][0] + 64],
        &fixture.bytes[fixture.records[0][0]..fixture.records[0][0] + 64]
    );
}

#[test]
fn paired_stream_save_without_changes_preserves_stream_and_sidecar() {
    let directory = TestDirectory::new();
    let stream_path = directory.path("paired.streamtex");
    let sidecar_path = directory.path("paired.textures");
    let first = dds(8, 8, 1);
    let second = dds(4, 4, 2);
    let original_stream = stream(&[first.clone(), second.clone()], b"");
    let sidecar = paired_sidecar(&[
        (0, (8, 8)),
        (0x1234_5678, (8, 8)),
        ((4 + first.len()) as u32, (4, 4)),
    ]);
    fs::write(&stream_path, &original_stream).unwrap();
    fs::write(&sidecar_path, &sidecar.bytes).unwrap();
    let file = load_streamtex_file(stream_path.clone()).unwrap();
    assert_eq!(&*file.dds_list[0].bytes, first);
    assert_eq!(&*file.dds_list[1].bytes, second);
    // The middle entry matches record 0's shape, so only the data-offset
    // binding can produce these names; a shift-based match would return the
    // middle entry's name for record 0.
    assert_eq!(file.dds_list[0].name, "00000001.dds");
    assert_eq!(file.dds_list[1].name, "00000003.dds");
    assert!(file.dds_list.iter().all(|dds| !dds.modified));
    save_streamtex_with_sidecar(&file, &stream_path, &sidecar_path).unwrap();
    assert_eq!(fs::read(&stream_path).unwrap(), original_stream);
    assert_eq!(fs::read(&sidecar_path).unwrap(), sidecar.bytes);
}

#[test]
fn paired_stream_replacement_updates_sidecar_offsets_and_record_headers() {
    let directory = TestDirectory::new();
    let stream_path = directory.path("replacement.streamtex");
    let sidecar_path = directory.path("replacement.textures");
    let first = dds(8, 8, 1);
    let second = dds(4, 4, 2);
    let sidecar = paired_sidecar(&[(0, (8, 8)), ((4 + first.len()) as u32, (4, 4))]);
    let original_sidecar = sidecar.bytes.clone();
    fs::write(&stream_path, stream(&[first, second.clone()], b"")).unwrap();
    fs::write(&sidecar_path, &sidecar.bytes).unwrap();
    let mut file = load_streamtex_file(stream_path.clone()).unwrap();
    assert_eq!(file.dds_list[0].name, "00000001.dds");
    let replacement = dds(16, 16, 5);
    file.dds_list[0].bytes = replacement.clone().into();
    file.dds_list[0].modified = true;
    save_streamtex_with_sidecar(&file, &stream_path, &sidecar_path).unwrap();
    assert_eq!(
        fs::read(&stream_path).unwrap(),
        stream(&[replacement.clone(), second.clone()], b"")
    );
    let mut expected_sidecar = original_sidecar;
    put_u32(&mut expected_sidecar, sidecar.records[0] + 24, 16);
    put_u32(&mut expected_sidecar, sidecar.records[0] + 28, 16);
    put_u32(
        &mut expected_sidecar,
        sidecar.entries[1] + 8,
        (4 + replacement.len()) as u32,
    );
    assert_eq!(fs::read(&sidecar_path).unwrap(), expected_sidecar);
    let reloaded = load_streamtex_file(stream_path).unwrap();
    assert_eq!(&*reloaded.dds_list[0].bytes, replacement);
    assert_eq!(&*reloaded.dds_list[1].bytes, second);
    assert_eq!(reloaded.dds_list[0].name, "00000001.dds");
    assert_eq!(reloaded.dds_list[1].name, "00000002.dds");
}
