//! Native EXIF via rawler's Rust TIFF library plus a small Sony MakerNote adapter.
//!
//! Sony tag definitions are documented by ExifTool's Sony.pm: 0xb04a is the
//! unsigned-short SequenceNumber; 0x202f is six-byte PixelShiftInfo (group u32,
//! shot u8, count u8). No subprocess or installed metadata utility is used.
//! Reference: https://github.com/exiftool/exiftool/blob/master/lib/Image/ExifTool/Sony.pm
use super::{Capture, PhotoMetadata};
use anyhow::{Context, Result, bail, ensure};
use rawler::{
    bits::Endian,
    formats::tiff::{GenericTiffReader, IFD, Value, reader::TiffReader},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PixelShiftInfo {
    pub group_id: u32,
    pub shot_number: u8,
    pub shot_count: u8,
}

pub fn metadata_backend() -> &'static str {
    "native-rust-rawler"
}

/// Inspect metadata without decoding the sensor or reading the complete RAW into RAM.
pub fn inspect_capture(path: &Path) -> Result<Capture> {
    let mut reader = File::open(path)?;
    read_capture(&mut reader)
        .with_context(|| format!("reading Sony metadata from {}", path.display()))
}

fn number(ifd: &IFD, tag: u16) -> Result<Option<f64>> {
    let Some(entry) = ifd.get_entry(tag) else {
        return Ok(None);
    };
    let value = match &entry.value {
        Value::Short(v) => v.first().copied().map(f64::from),
        Value::Long(v) => v.first().copied().map(f64::from),
        Value::Rational(v) => match v.first() {
            Some(v) if v.d != 0 => Some(f64::from(v.n) / f64::from(v.d)),
            _ => bail!("invalid rational EXIF tag 0x{tag:04x}"),
        },
        _ => bail!("invalid numeric EXIF tag 0x{tag:04x}"),
    };
    ensure!(value.is_some(), "empty numeric EXIF tag 0x{tag:04x}");
    Ok(value)
}

fn pixel_shift(maker: &IFD) -> Result<Option<PixelShiftInfo>> {
    let Some(entry) = maker.get_entry(0x202fu16) else {
        return Ok(None);
    };
    let data = match &entry.value {
        Value::Undefined(v) | Value::Byte(v) => v,
        _ => bail!("invalid Sony PixelShiftInfo type"),
    };
    ensure!(
        data.len() == 6,
        "Sony PixelShiftInfo must contain six bytes"
    );
    if data.iter().all(|v| *v == 0) {
        return Ok(None);
    }
    let group_bytes: [u8; 4] = data[..4].try_into()?;
    let group_id = match maker.endian {
        Endian::Little => u32::from_le_bytes(group_bytes),
        Endian::Big => u32::from_be_bytes(group_bytes),
    };
    let shot_number = data[4];
    let shot_count = data[5];
    ensure!(
        matches!(shot_count, 4 | 16),
        "unsupported Sony Pixel Shift shot count {shot_count}"
    );
    ensure!(
        shot_number > 0 && shot_number <= shot_count,
        "Sony PixelShiftInfo is not a source shot (index {shot_number})"
    );
    Ok(Some(PixelShiftInfo {
        group_id,
        shot_number,
        shot_count,
    }))
}

/// Sony ARW MakerNoteSony5 starts directly with an IFD. DSC/CAM variants have
/// a 12-byte prefix. All retain TIFF-relative absolute value offsets.
/// Reference: ExifTool MakerNotes.pm, MakerNoteSony and MakerNoteSony5.
fn sony_maker_note<R: Read + Seek>(parent: &IFD, reader: &mut R) -> Result<IFD> {
    let entry = parent.get_entry(0x927cu16).context("missing MakerNote")?;
    let bytes = match &entry.value {
        Value::Undefined(bytes) => bytes,
        _ => bail!("invalid Sony MakerNote type"),
    };
    let prefix = bytes
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut skip = if bytes.starts_with(b"SONY DSC ") || bytes.starts_with(b"SONY CAM ") {
        12
    } else {
        0
    };
    let mut endian = parent.endian;
    if bytes.get(skip..skip + 2) == Some(b"II") {
        endian = Endian::Little;
        skip += 2;
    } else if bytes.get(skip..skip + 2) == Some(b"MM") {
        endian = Endian::Big;
        skip += 2;
    }
    let raw_count: [u8; 2] = bytes
        .get(skip..skip + 2)
        .with_context(|| {
            format!(
                "truncated Sony MakerNote (length={}, prefix={prefix})",
                bytes.len()
            )
        })?
        .try_into()?;
    let count = match endian {
        Endian::Little => u16::from_le_bytes(raw_count),
        Endian::Big => u16::from_be_bytes(raw_count),
    } as usize;
    ensure!(
        count > 0 && skip + 2 + count * 12 <= bytes.len(),
        "invalid/truncated Sony MakerNote directory (length={}, entries={count}, prefix={prefix})",
        bytes.len()
    );
    let offset = u32::try_from(entry.offset().context("Sony MakerNote has no offset")?)?
        .checked_add(u32::try_from(skip)?)
        .context("Sony MakerNote offset overflow")?;
    IFD::new(reader, offset, parent.base, parent.corr, endian, &[])
        .context("parsing Sony MakerNote directory")
}

fn read_capture<R: Read + Seek>(reader: &mut R) -> Result<Capture> {
    let tiff =
        GenericTiffReader::new(reader, 0, 0, Some(4), &[]).context("parsing RAW TIFF/EXIF")?;
    let root = tiff.root_ifd();
    let make = root
        .get_entry(0x010fu16)
        .and_then(|e| e.value.as_string())
        .context("missing camera make")?;
    ensure!(
        make.trim().eq_ignore_ascii_case("SONY"),
        "native capture metadata supports Sony ARW only"
    );
    let model = root
        .get_entry(0x0110u16)
        .and_then(|e| e.value.as_string())
        .map(|s| s.trim().to_owned());
    let dimensions = root
        .find_ifds_with_tag(0x0100u16)
        .into_iter()
        .filter_map(|ifd| match (number(ifd, 0x0100), number(ifd, 0x0101)) {
            (Ok(Some(w)), Ok(Some(h))) if w > 0.0 && h > 0.0 => Some((w as usize, h as usize)),
            _ => None,
        })
        .max_by_key(|(w, h)| w.saturating_mul(*h));
    let exif = root
        .find_first_ifd_with_tag(0x829au16)
        .context("missing exposure EXIF IFD")?;
    let exposure = number(exif, 0x829a)?.context("missing exposure time")?;
    let aperture = number(exif, 0x829d)?.context("missing FNumber")?;
    let iso = number(exif, 0x8827)?
        .or(number(exif, 0x8833)?)
        .context("missing ISO")?;
    ensure!(
        exposure > 0.0 && aperture > 0.0 && iso > 0.0,
        "invalid exposure metadata"
    );
    let serial = exif
        .get_entry(0xa431u16)
        .and_then(|e| e.value.as_string())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let maker_parent = root.find_first_ifd_with_tag(0x927cu16);
    let (sequence, pixel_shift) = if let Some(parent) = maker_parent {
        let maker = sony_maker_note(parent, reader)?;
        let shift = pixel_shift(&maker)?;
        let sequence = if let Some(info) = &shift {
            Some(usize::from(info.shot_number))
        } else {
            let sequence = number(&maker, 0xb04a)?.map(|v| v as usize);
            sequence.filter(|n| *n > 0 && *n != 65535)
        };
        (sequence, shift)
    } else {
        bail!("missing Sony MakerNote")
    };
    let standard = rawler::exif::Exif::new(root)?;
    let photo_metadata = PhotoMetadata {
        make: Some(make.trim().to_owned()),
        model: model.clone(),
        lens_make: standard.lens_make,
        lens_model: standard.lens_model,
        lens_spec: standard.lens_spec,
        focal_length: standard.focal_length,
        exposure_time: standard.exposure_time,
        fnumber: standard.fnumber,
        iso: standard
            .iso_speed
            .or(standard.iso_speed_ratings.map(u32::from)),
        date_time_original: standard.date_time_original,
    };
    Ok(Capture {
        photo_metadata,
        model,
        dimensions,
        sequence,
        iso: Some(iso),
        exposure: Some(exposure),
        aperture: Some(aperture),
        serial,
        pixel_shift,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    // A real TIFF hierarchy with absolute-offset Sony MakerNotes, in either byte
    // order. It exercises rawler's library parser, not a mock Capture record.
    fn fixture(big: bool, shift: [u8; 6], sequence: u16) -> Vec<u8> {
        let mut b = vec![0u8; 800];
        fn short(b: &mut [u8], p: usize, v: u16, big: bool) {
            b[p..p + 2].copy_from_slice(&if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            });
        }
        fn long(b: &mut [u8], p: usize, v: u32, big: bool) {
            b[p..p + 4].copy_from_slice(&if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            });
        }
        fn entry(b: &mut [u8], p: usize, tag: u16, kind: u16, count: u32, value: u32, big: bool) {
            short(b, p, tag, big);
            short(b, p + 2, kind, big);
            long(b, p + 4, count, big);
            if kind == 3 && count == 1 {
                short(b, p + 8, value as u16, big)
            } else {
                long(b, p + 8, value, big)
            }
        }
        b[..2].copy_from_slice(if big { b"MM" } else { b"II" });
        short(&mut b, 2, 42, big);
        long(&mut b, 4, 272, big);
        short(&mut b, 272, 5, big);
        entry(&mut b, 274, 0x0100, 4, 1, 9504, big);
        entry(&mut b, 286, 0x0101, 4, 1, 6336, big);
        entry(&mut b, 298, 0x010f, 2, 5, 40, big);
        entry(&mut b, 310, 0x0110, 2, 9, 256, big);
        entry(&mut b, 322, 0x8769, 4, 1, 64, big);
        b[256..265].copy_from_slice(b"ILCE-7CR\0");
        b[40..45].copy_from_slice(b"SONY\0");
        short(&mut b, 64, 5, big);
        entry(&mut b, 66, 0x829a, 5, 1, 144, big);
        entry(&mut b, 78, 0x829d, 5, 1, 152, big);
        entry(&mut b, 90, 0x8827, 3, 1, 100, big);
        entry(&mut b, 102, 0x927c, 7, 54, 192, big);
        entry(&mut b, 114, 0xa431, 2, 9, 160, big);
        long(&mut b, 144, 1, big);
        long(&mut b, 148, 100, big);
        long(&mut b, 152, 8, big);
        long(&mut b, 156, 1, big);
        b[160..169].copy_from_slice(b"camera-x\0");
        b[192..204].copy_from_slice(b"SONY DSC \0\0\0");
        short(&mut b, 204, 2, big);
        entry(&mut b, 206, 0x202f, 7, 6, 240, big);
        entry(&mut b, 218, 0xb04a, 3, 1, u32::from(sequence), big);
        b[240..246].copy_from_slice(&shift);
        // Standard EXIF alongside Sony MakerNotes; preserve lens identity without
        // copying the private MakerNote into a rendered output.
        b.copy_within(64..126, 400);
        long(&mut b, 330, 400, big);
        short(&mut b, 400, 8, big);
        entry(&mut b, 462, 0xa433, 2, 5, 600, big);
        entry(&mut b, 474, 0xa434, 2, 14, 620, big);
        entry(&mut b, 486, 0x920a, 5, 1, 660, big);
        b[600..605].copy_from_slice(b"Sony\0");
        b[620..634].copy_from_slice(b"FE 20-70mm F4\0");
        long(&mut b, 660, 35, big);
        long(&mut b, 664, 1, big);
        b
    }
    #[test]
    fn native_tiff_sony_capture_in_both_byte_orders() {
        for big in [false, true] {
            let group = if big {
                42u32.to_be_bytes()
            } else {
                42u32.to_le_bytes()
            };
            let mut shift = [0; 6];
            shift[..4].copy_from_slice(&group);
            shift[4] = 3;
            shift[5] = 16;
            let capture = read_capture(&mut Cursor::new(fixture(big, shift, 10))).unwrap();
            assert_eq!(capture.sequence, Some(3));
            assert_eq!(capture.model.as_deref(), Some("ILCE-7CR"));
            assert_eq!(capture.dimensions, Some((9504, 6336)));
            assert_eq!(capture.photo_metadata.make.as_deref(), Some("SONY"));
            assert_eq!(capture.photo_metadata.lens_make.as_deref(), Some("Sony"));
            assert_eq!(
                capture.photo_metadata.lens_model.as_deref(),
                Some("FE 20-70mm F4")
            );
            assert_eq!(capture.photo_metadata.focal_length.unwrap().n, 35);
            assert_eq!(capture.iso, Some(100.));
            assert_eq!(capture.exposure, Some(0.01));
            assert_eq!(capture.aperture, Some(8.));
            assert_eq!(capture.serial.as_deref(), Some("camera-x"));
            assert_eq!(capture.pixel_shift.unwrap().group_id, 42);
        }
    }
    fn headerless(big: bool, shift: [u8; 6]) -> Vec<u8> {
        let mut bytes = fixture(big, shift, 3);
        // Copy the IFD entry table, not the out-of-line value payload.
        bytes.copy_within(204..234, 192);
        bytes[442..446].copy_from_slice(&if big {
            42u32.to_be_bytes()
        } else {
            42u32.to_le_bytes()
        });
        bytes
    }
    #[test]
    fn reads_headerless_arw_and_cam_variants() {
        for big in [false, true] {
            let mut shift = [0; 6];
            shift[..4].copy_from_slice(&if big {
                42u32.to_be_bytes()
            } else {
                42u32.to_le_bytes()
            });
            shift[4] = 3;
            shift[5] = 16;
            let capture = read_capture(&mut Cursor::new(headerless(big, shift))).unwrap();
            assert_eq!(capture.sequence, Some(3));
            assert_eq!(capture.pixel_shift.unwrap().group_id, 42);
            let mut cam = fixture(big, shift, 3);
            cam[192..204].copy_from_slice(b"SONY CAM \0\0\0");
            assert_eq!(
                read_capture(&mut Cursor::new(cam)).unwrap().sequence,
                Some(3)
            );
        }
    }
    #[test]
    fn rejects_unknown_and_truncated_maker_directories() {
        let mut unknown = fixture(false, [0; 6], 1);
        unknown[192..204].copy_from_slice(b"UNKNOWN DATA");
        assert!(
            read_capture(&mut Cursor::new(unknown))
                .unwrap_err()
                .to_string()
                .contains("prefix=")
        );
        let mut bad = headerless(false, [0; 6]);
        bad[192..194].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(read_capture(&mut Cursor::new(bad)).is_err());
    }
    #[test]
    fn sequence_fallback_and_single_burst() {
        for (index, expected) in [(7, Some(7)), (0, None), (65535, None)] {
            let capture = read_capture(&mut Cursor::new(fixture(false, [0; 6], index))).unwrap();
            assert_eq!(capture.sequence, expected);
            assert!(capture.pixel_shift.is_none());
        }
    }
    #[test]
    fn rejects_composed_or_malformed_pixel_shift() {
        for shift in [
            [1, 0, 0, 0, 0, 16],
            [1, 0, 0, 0, 17, 16],
            [1, 0, 0, 0, 1, 8],
        ] {
            assert!(read_capture(&mut Cursor::new(fixture(false, shift, 1))).is_err());
        }
    }
    #[test]
    fn truncated_metadata_fails() {
        for size in [0, 8, 20, 64, 192, 220] {
            let mut bytes = fixture(false, [0; 6], 1);
            bytes.truncate(size);
            assert!(read_capture(&mut Cursor::new(bytes)).is_err());
        }
    }
}
