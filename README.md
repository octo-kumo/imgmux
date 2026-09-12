# imgmux

Store multiple images - PNG, JPEG, GIF, and WebP - in a single file, plus metadata, without breaking any of them. The trick: image decoders ignore bytes appended past the end of an image, so the file always starts with a complete, valid image and the extra data lives beyond its end.

The current image sits at offset 0, so the mux *is* a valid image: `file`, browsers, and image viewers all show the current one. Metadata and every other image follow after the current image's end. Switching images rewrites the file with the selected image moved to offset 0.

```
[ current image bytes ][ meta ][ other images... ]
                        ^ found by parsing the image at offset 0
```

## Build

```sh
cargo build --release
# binary: target/release/imgmux
```

Requires only `blake3` and `memmap2`. Run the test suite with `cargo test`.

## Usage

```
imgmux <image>... <out.mux>    create mux, or add if out.mux is a valid mux
imgmux <image>... > out.mux    create mux on stdout when stdout is piped
imgmux <mux>                   print metadata
imgmux explode <mux>           extract all images to their original names
imgmux -s=N <mux>              switch current image to index N
imgmux -d=N <mux>              delete image N (cannot be the current one)
imgmux -c[=N] <mux>            cycle current index by N (default 1, may be negative)
```

Options may be written `-s=2`, `-s 2`, or `--switch=2` (same for `-d/--delete`,
`-c/--cycle`).

### Examples

```sh
# Build a mux from three images (out.mux is a valid PNG)
imgmux logo.png photo.jpg art.webp out.mux

# Pipe-friendly: same thing to stdout
imgmux logo.png photo.jpg > out.mux

# Show metadata, including names, sizes, formats, and timestamps
imgmux out.mux

# Make image 2 current (the file is rewritten atomically)
imgmux -s=2 out.mux

# Add another image; the current image is preserved and new images append
imgmux extra.gif out.mux

# Cycle backward through images
imgmux -c=-1 out.mux

# Delete image 0 (not allowed if it is current)
imgmux -d=0 out.mux

# Extract every image to its original filename
imgmux explode out.mux
```

## Meta format (version 1, little-endian)

| field          | type      | notes                                        |
| -------------- | --------- | -------------------------------------------- |
| magic          | `[u8; 4]` | `49 4D 58 A7` (`"IMX"` + `0xA7`)             |
| version        | `u16`     | currently `1`                                |
| flags          | `u16`     | must be `0`                                  |
| count          | `u32`     | total number of images                       |
| current        | `u32`     | index of the current image                   |
| last_changed   | `u64`     | Unix seconds of the last mutation            |
| last_added     | `u64`     | Unix seconds of the last image addition      |
| checksum       | `[u8; 32]`| BLAKE3 of the whole file with these 32 bytes zeroed |
| entries        | `count *` | see below                                    |
| tail_len       | `u32`     | length of the reserved tail                  |
| tail           | bytes     | reserved for future fields (empty in v1)     |

Each entry:

| field     | type      | notes                                 |
| --------- | --------- | ------------------------------------- |
| offset    | `u64`     | absolute file offset                  |
| size      | `u64`     | image length in bytes                 |
| added     | `u64`     | Unix seconds when the image was added |
| flags     | `u16`     | must be `0` (reserved)                |
| name_len  | `u16`     | length of the UTF-8 filename          |
| name      | bytes     | original basename                     |

Entries are kept in append order and never reordered; `current` points into that list. Physically the current image is at offset 0, and the remaining images follow the meta in entry order.

## Validation

Opening a mux performs full, strict validation and fails hard on any mismatch:

1. Parse the image at offset 0 to get its exact end offset (no pixel decoding).
2. Verify the meta magic at that offset and deserialize the meta.
3. Verify the current entry has `offset == 0` and `size ==` the parsed size.
4. Verify non-current images are contiguous after the meta, in entry order, in bounds, and that each one re-parses to exactly its recorded size.
5. Verify the file ends exactly at the last image (no gaps or trailing bytes).
6. Recompute the BLAKE3 checksum over the whole file (checksum field zeroed).

## Atomicity

Every mutation (`switch`, `add`, `delete`, `cycle`) writes a complete new file to a temp file in the same directory, fsyncs it, atomically renames it over the original, and fsyncs the parent directory. The result is either the fully updated mux or the untouched original.

## Notes

- All mutating operations validate the container first.
- Input files with bytes past the image end are rejected (`N trailing byte(s) after PNG image`), because that data would collide with the meta placement.
- `explode` checks every target before writing: identical files are skipped, and a differing existing file aborts the whole extraction.
- Filenames are stored as basenames; path separators are rejected on read.
