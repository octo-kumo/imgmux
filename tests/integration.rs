use imgmux::container::{validate, Container};
use imgmux::format;
use imgmux::ops::{self, Out};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn tmpdir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("imgmux-test-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn png() -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(&0u32.to_be_bytes());
    b.extend_from_slice(b"IEND");
    b.extend_from_slice(&[0xAE, 0x42, 0x60, 0x82]);
    b
}

fn jpeg() -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8];
    b.extend_from_slice(&[0xFF, 0xE0]);
    let payload = b"JFIF\0\x01\x01\x00\x00\x01\x00\x01\x00\x00";
    b.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    b.extend_from_slice(payload);
    b.extend_from_slice(&[0xFF, 0xC0]);
    b.extend_from_slice(&17u16.to_be_bytes());
    b.extend_from_slice(&[8, 0, 1, 0, 1, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
    b.extend_from_slice(&[0xFF, 0xDA]);
    b.extend_from_slice(&12u16.to_be_bytes());
    b.extend_from_slice(&[3, 1, 0, 2, 0, 3, 0, 0, 0x3F, 0]);
    b.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34]);
    b.extend_from_slice(&[0xFF, 0xD9]);
    b
}

fn gif() -> Vec<u8> {
    let mut b = b"GIF89a".to_vec();
    b.extend_from_slice(&[1, 0, 1, 0, 0x00, 0, 0]);
    b.push(0x2C);
    b.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0, 0x00]);
    b.push(0x02);
    b.push(0x01);
    b.push(0x00);
    b.push(0x00);
    b.push(0x3B);
    b
}

fn webp() -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(b"WEBP");
    b.extend_from_slice(b"VP8L");
    let data = [0x2F, 0, 0, 0, 0];
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(&data);
    b.push(0);
    let total = b.len();
    b[4..8].copy_from_slice(&((total - 8) as u32).to_le_bytes());
    b
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}

fn setup(dir: &Path) -> (PathBuf, Vec<u8>, PathBuf, Vec<u8>, PathBuf, Vec<u8>) {
    let a = png();
    let b = jpeg();
    let c = gif();
    (
        write(dir, "a.png", &a),
        a,
        write(dir, "b.jpg", &b),
        b,
        write(dir, "c.gif", &c),
        c,
    )
}

#[test]
fn create_validate_and_info() {
    let dir = tmpdir("create");
    let (a, ab, b, bb, c, cb) = setup(&dir);
    let mux = dir.join("m.mux");
    ops::create(&[a, b, c], Out::File(mux.clone())).unwrap();

    let bytes = fs::read(&mux).unwrap();
    let (fmt, end) = format::end_of_image(&bytes).unwrap();
    assert_eq!(fmt, format::Format::Png);
    assert!(end < bytes.len());

    let cont = Container::open(&mux).unwrap();
    assert_eq!(cont.count(), 3);
    assert_eq!(cont.current(), 0);
    assert_eq!(cont.image(0), &ab[..]);
    assert_eq!(cont.image(1), &bb[..]);
    assert_eq!(cont.image(2), &cb[..]);
    assert_eq!(cont.current_format, format::Format::Png);

    let mut out = Vec::new();
    ops::info(&cont, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("3 image(s)"));
    assert!(text.contains("a.png"));
    assert!(text.contains("b.jpg"));
    assert!(text.contains("c.gif"));
    assert!(text.contains("checksum: ok"));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn switch_and_cycle() {
    let dir = tmpdir("switch");
    let (a, ab, b, bb, c, cb) = setup(&dir);
    let mux = dir.join("m.mux");
    ops::create(&[a, b, c], Out::File(mux.clone())).unwrap();

    let cont = Container::open(&mux).unwrap();
    assert_eq!(ops::switch(&cont, 2).unwrap(), Some(2));
    drop(cont);

    let cont = Container::open(&mux).unwrap();
    assert_eq!(cont.current(), 2);
    assert_eq!(cont.image(0), &ab[..]);
    assert_eq!(cont.image(1), &bb[..]);
    assert_eq!(cont.image(2), &cb[..]);
    assert_eq!(cont.current_format, format::Format::Gif);
    // the current image is physically first
    assert_eq!(
        format::end_of_image(cont.data()).unwrap(),
        (format::Format::Gif, cb.len())
    );

    assert_eq!(ops::switch(&cont, 2).unwrap(), None);
    assert_eq!(ops::cycle(&cont, -1).unwrap(), Some(1));
    drop(cont);

    let cont = Container::open(&mux).unwrap();
    assert_eq!(cont.current(), 1);
    assert_eq!(cont.image(0), &ab[..]);
    assert_eq!(cont.image(1), &bb[..]);
    assert_eq!(
        format::end_of_image(cont.data()).unwrap(),
        (format::Format::Jpeg, bb.len())
    );
    assert_eq!(ops::cycle(&cont, 3).unwrap(), None); // mod count wraps to self
    assert!(ops::switch(&cont, 99).is_err());

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn add_preserves_current_and_appends() {
    let dir = tmpdir("add");
    let (a, ab, b, bb, c, cb) = setup(&dir);
    let mux = dir.join("m.mux");
    ops::create(&[a, b], Out::File(mux.clone())).unwrap();

    let cont = Container::open(&mux).unwrap();
    assert_eq!(ops::switch(&cont, 1).unwrap(), Some(1));
    drop(cont);

    let cont = Container::open(&mux).unwrap();
    ops::add(&cont, &[c]).unwrap();
    drop(cont);

    let cont = Container::open(&mux).unwrap();
    assert_eq!(cont.count(), 3);
    assert_eq!(cont.current(), 1);
    assert_eq!(cont.image(0), &ab[..]);
    assert_eq!(cont.image(1), &bb[..]);
    assert_eq!(cont.image(2), &cb[..]);
    assert_eq!(
        format::end_of_image(cont.data()).unwrap(),
        (format::Format::Jpeg, bb.len())
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn delete_rules_and_index_shift() {
    let dir = tmpdir("delete");
    let (a, _ab, b, bb, c, cb) = setup(&dir);
    let mux = dir.join("m.mux");
    ops::create(&[a, b, c], Out::File(mux.clone())).unwrap();

    let cont = Container::open(&mux).unwrap();
    assert!(ops::delete(&cont, 0).is_err()); // current
    drop(cont);

    let cont = Container::open(&mux).unwrap();
    assert_eq!(ops::switch(&cont, 2).unwrap(), Some(2));
    drop(cont);

    let cont = Container::open(&mux).unwrap();
    ops::delete(&cont, 0).unwrap();
    drop(cont);

    let cont = Container::open(&mux).unwrap();
    assert_eq!(cont.count(), 2);
    assert_eq!(cont.current(), 1);
    assert_eq!(cont.image(0), &bb[..]);
    assert_eq!(cont.image(1), &cb[..]);
    assert_eq!(cont.meta.entries[0].name, "b.jpg");
    assert_eq!(cont.meta.entries[1].name, "c.gif");

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn explode_skips_identical_and_errors_on_conflict() {
    let dir = tmpdir("explode");
    let out = dir.join("out");
    fs::create_dir(&out).unwrap();
    let (a, ab, b, bb, c, _cb) = setup(&dir);
    let mux = dir.join("m.mux");
    ops::create(&[a, b, c], Out::File(mux.clone())).unwrap();

    let cont = Container::open(&mux).unwrap();
    ops::explode(&cont, &out).unwrap();
    assert_eq!(fs::read(out.join("a.png")).unwrap(), ab);
    assert_eq!(fs::read(out.join("b.jpg")).unwrap(), bb);

    // identical files are skipped without error
    ops::explode(&cont, &out).unwrap();

    // conflicting content fails and writes nothing else
    fs::write(out.join("a.png"), b"different").unwrap();
    fs::remove_file(out.join("b.jpg")).unwrap();
    assert!(ops::explode(&cont, &out).is_err());
    assert!(!out.join("b.jpg").exists());

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn corruption_and_junk_are_rejected() {
    let dir = tmpdir("corrupt");
    let (a, _ab, b, _bb, c, _cb) = setup(&dir);
    let mux = dir.join("m.mux");
    ops::create(&[a, b, c], Out::File(mux.clone())).unwrap();
    let good = fs::read(&mux).unwrap();
    validate(&good).unwrap();

    let mut bad = good.clone();
    bad[20] ^= 0x40; // inside current image data
    assert!(validate(&bad).is_err());

    let mut bad = good.clone();
    let meta_start = format::end_of_image(&good).unwrap().1;
    bad[meta_start + 40] ^= 0x01; // entry data / filename
    assert!(validate(&bad).is_err());

    let mut bad = good.clone();
    bad.push(0); // trailing junk
    assert!(validate(&bad).is_err());

    let bad = &good[..good.len() - 1];
    assert!(validate(bad).is_err());

    // input files with trailing bytes are not accepted as images
    let junk = write(&dir, "junk.png", &{
        let mut v = png();
        v.extend_from_slice(b"trailing");
        v
    });
    assert!(ops::create(&[junk], Out::File(dir.join("x.mux"))).is_err());

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cli_pipe_mode_writes_valid_mux_to_stdout() {
    let dir = tmpdir("cli");
    let (a, _ab, b, _bb, c, _cb) = setup(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_imgmux"))
        .args([
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            c.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    validate(&out.stdout).unwrap();

    // the mux bytes are a valid image prefix followed by meta magic
    let (_, end) = format::end_of_image(&out.stdout).unwrap();
    assert_eq!(&out.stdout[end..end + 4], &[0x49, 0x4D, 0x58, 0xA7]);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn webp_container_roundtrip() {
    let dir = tmpdir("webp");
    let w = webp();
    let path = write(&dir, "w.webp", &w);
    let mux = dir.join("w.mux");
    ops::create(&[path], Out::File(mux.clone())).unwrap();
    let cont = Container::open(&mux).unwrap();
    assert_eq!(cont.count(), 1);
    assert_eq!(cont.current_format, format::Format::WebP);
    assert_eq!(cont.image(0), &w[..]);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn real_sample_images_roundtrip() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inputs: Vec<PathBuf> = ["logo.png", "142303704_p0.jpg", "46_morning_4k.jpg"]
        .iter()
        .map(|n| dir.join(n))
        .filter(|p| p.exists())
        .collect();
    if inputs.len() < 3 {
        return; // samples not present
    }
    let out = tmpdir("real");
    let mux = out.join("real.mux");
    ops::create(&inputs, Out::File(mux.clone())).unwrap();
    let cont = Container::open(&mux).unwrap();
    assert_eq!(cont.count(), 3);
    for (i, p) in inputs.iter().enumerate() {
        assert_eq!(cont.image(i), fs::read(p).unwrap());
    }
    fs::remove_dir_all(&out).unwrap();
}
