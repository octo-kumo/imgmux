use crate::container::{assemble, write_container, Container, Image, ImageData};
use crate::format;
use crate::meta::VERSION;
use crate::util::{atomic_write, fmt_unix, now};
use crate::{inv, Res};
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

pub enum Out {
    Stdout,
    File(PathBuf),
}

/// Read a plain image file, rejecting files with trailing bytes past the image end.
pub fn read_image_file(path: &Path) -> Res<(String, Vec<u8>)> {
    let bytes = fs::read(path).map_err(|e| inv(format!("{}: {e}", path.display())))?;
    let (fmt, end) =
        format::end_of_image(&bytes).map_err(|e| inv(format!("{}: {e}", path.display())))?;
    if end != bytes.len() {
        return Err(inv(format!(
            "{}: {} trailing byte(s) after {} image; not a plain image",
            path.display(),
            bytes.len() - end,
            fmt.name()
        )));
    }
    let name = path
        .file_name()
        .ok_or_else(|| inv(format!("{}: not a file name", path.display())))?
        .to_string_lossy()
        .into_owned();
    Ok((name, bytes))
}

fn write_out(
    out: Out,
    imgs: &[Image<'_>],
    current: usize,
    last_changed: u64,
    last_added: u64,
    tail: &[u8],
) -> Res<()> {
    let (_, meta_bytes) = assemble(imgs, current, last_changed, last_added, tail)?;
    match out {
        Out::Stdout => {
            let stdout = io::stdout();
            let mut w = BufWriter::new(stdout.lock());
            write_container(&mut w, imgs, current, &meta_bytes)?;
            w.flush()?;
        }
        Out::File(path) => {
            atomic_write(&path, |f| {
                write_container(f, imgs, current, &meta_bytes)?;
                Ok(())
            })?;
        }
    }
    Ok(())
}

pub fn create(paths: &[PathBuf], out: Out) -> Res<()> {
    if paths.is_empty() {
        return Err(inv("no input images"));
    }
    let mut imgs: Vec<Image<'_>> = Vec::with_capacity(paths.len());
    for p in paths {
        let (name, data) = read_image_file(p)?;
        imgs.push(Image {
            name,
            added: now(),
            data: ImageData::Owned(data),
        });
    }
    let t = now();
    write_out(out, &imgs, 0, t, t, &[])
}

pub fn add(c: &Container, paths: &[PathBuf]) -> Res<()> {
    if paths.is_empty() {
        return Err(inv("no images to add"));
    }
    let mut imgs = c.images();
    for p in paths {
        let (name, data) = read_image_file(p)?;
        imgs.push(Image {
            name,
            added: now(),
            data: ImageData::Owned(data),
        });
    }
    let t = now();
    write_out(
        Out::File(c.path.clone()),
        &imgs,
        c.current(),
        t,
        t,
        &c.meta.tail,
    )
}

pub fn switch(c: &Container, n: usize) -> Res<Option<usize>> {
    if n >= c.count() {
        return Err(inv(format!(
            "image index {n} out of range (0..{})",
            c.count() - 1
        )));
    }
    if n == c.current() {
        return Ok(None);
    }
    let imgs = c.images();
    write_out(
        Out::File(c.path.clone()),
        &imgs,
        n,
        now(),
        c.meta.last_added,
        &c.meta.tail,
    )?;
    Ok(Some(n))
}

pub fn delete(c: &Container, n: usize) -> Res<()> {
    if n >= c.count() {
        return Err(inv(format!(
            "image index {n} out of range (0..{})",
            c.count() - 1
        )));
    }
    if n == c.current() {
        return Err(inv("cannot delete the current image; switch first"));
    }
    let mut imgs = c.images();
    imgs.remove(n);
    let mut current = c.current();
    if n < current {
        current -= 1;
    }
    write_out(
        Out::File(c.path.clone()),
        &imgs,
        current,
        now(),
        c.meta.last_added,
        &c.meta.tail,
    )
}

pub fn cycle(c: &Container, delta: i64) -> Res<Option<usize>> {
    let count = c.count() as i64;
    let cur = c.current() as i64;
    let next = ((cur + delta) % count + count) % count;
    switch(c, next as usize)
}

pub fn explode(c: &Container, dir: &Path) -> Res<()> {
    let mut writes: Vec<(PathBuf, usize)> = Vec::new();
    for (i, e) in c.meta.entries.iter().enumerate() {
        let target = dir.join(&e.name);
        if target.exists() {
            let existing =
                fs::read(&target).map_err(|err| inv(format!("{}: {err}", target.display())))?;
            if existing == c.image(i) {
                eprintln!("skip {} (identical)", target.display());
                continue;
            }
            return Err(inv(format!(
                "{} already exists and differs (image {i})",
                target.display()
            )));
        }
        writes.push((target, i));
    }
    for (target, i) in writes {
        atomic_write(&target, |f| {
            f.write_all(c.image(i))?;
            Ok(())
        })?;
        eprintln!("wrote {}", target.display());
    }
    Ok(())
}

pub fn info(c: &Container, out: &mut impl Write) -> Res<()> {
    writeln!(
        out,
        "imgmux v{VERSION} | {} image(s) | current: {} ({})",
        c.count(),
        c.current(),
        c.meta.entries[c.current()].name
    )?;
    writeln!(out, "last changed: {}", fmt_unix(c.meta.last_changed))?;
    writeln!(out, "last added:   {}", fmt_unix(c.meta.last_added))?;
    for (i, e) in c.meta.entries.iter().enumerate() {
        let fmt = format::detect(c.image(i)).map(|f| f.name()).unwrap_or("?");
        writeln!(
            out,
            "{}[{i}] {}  {fmt}  {} B  added {}",
            if i == c.current() { "*" } else { " " },
            e.name,
            e.size,
            fmt_unix(e.added)
        )?;
    }
    writeln!(out, "checksum: ok (blake3)")?;
    Ok(())
}
