use anyhow::{anyhow, Result};
use clap::{Arg, App};
use indexmap::IndexMap;
use memchr::memmem;
use serde::Deserialize;
use serde_json::{Deserializer, Value};
use std::fs;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::str;
use std::io::Write;

use cap_std::ambient_authority;
use cap_std::fs::Dir;

fn get_float_from_buf(buf: &[u8], index: usize) -> Result<usize> {
    let val: &[u8; 8] = buf.get(index..index+8)
            .ok_or(anyhow!("Truncated nexe metadata"))?.try_into()?;
    Ok(f64::from_le_bytes(*val) as usize)
}

#[derive(Deserialize, Debug)]
struct Offsets {
    start_offset: usize,
    end_offset: usize
}

type Resources = IndexMap<String, Offsets>;

type Dictionary = IndexMap<String, Resources>;

impl fmt::Display for Offsets {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "[{}, {}]", self.start_offset, self.end_offset)
    }
}

fn main() -> Result<()> {
    let matches = App::new("nexe-unpacker")
        .version("1.0.0")
        .arg(Arg::new("file")
            .required(true))
        .arg(Arg::new("output")
            .default_value("out"))
        .get_matches();

    let file: PathBuf = matches.value_of_t_or_exit("file");
    //println!("Opening file: {}", file.display());

    let output_path: PathBuf = matches.value_of_t_or_exit("output");
    Dir::create_ambient_dir_all(&output_path, ambient_authority())?;
    let output_dir = Dir::open_ambient_dir(&output_path, ambient_authority())?;

    let binary: Vec<u8> = fs::read(file)?;
    const SENTINEL_VAL: &[u8; 16] = b"<nexe~~sentinel>";
    let footer_position = memmem::rfind(&binary, SENTINEL_VAL)
        .ok_or(anyhow!("Invalid nexe executable"))?;
    //println!("Footer Position: {footer_position} 0x{footer_position:x}");
    let content_size: usize = get_float_from_buf(&binary, footer_position+SENTINEL_VAL.len())?;
    //println!("Content size: {content_size}");

    let resource_size: usize = get_float_from_buf(&binary, footer_position+SENTINEL_VAL.len()+8)?;
    //println!("Resource size: {resource_size}");

    let content_start: usize = footer_position - resource_size - content_size;
    //println!("Content start: {content_start}");

    let resource_start: usize = content_start + content_size;
    //println!("Resource start: {resource_start}");

    //println!("Content:");
    //println!("{}", str::from_utf8(&binary[content_start..content_start+content_size])?);

    //println!("Resource:");
    //println!("{:?}", &binary[resource_start..resource_start+resource_size]);

    let content = &binary[content_start..content_start+content_size];
    const CONTENT_MARKER: &[u8; 32] = b"!(function () {process.__nexe = ";
    let resource_metadata_start = memmem::find(content, CONTENT_MARKER)
        .ok_or(anyhow!("Invalid content section"))? + CONTENT_MARKER.len();

    let mut stream = Deserializer::from_slice(&content[resource_metadata_start..]).into_iter::<Value>();
    let v = stream.next()
        .ok_or(anyhow!("Unable to parse JSON metadata"))??;
    let d: Dictionary = serde_json::from_value(v)?;
    let res: &Resources = &d["resources"];
    //println!("Res: {:?}", res);
    for (filename, offsets) in res.iter() {
        // HACK: fixup Windows path separators
        let filename = filename.replace("\\", "/");
        // HACK: badly remove ../ to make extraction not fail for paths that begin with ../
        //       cap-std will block actual directory traversal attempts
        let filename = filename.replace("../", "");
        println!("Filename: {}", filename);
        println!("Offsets: {}", offsets);
        let filepath = Path::new(&filename);

        let resource = &binary[resource_start..resource_start+resource_size];
        let payload_data = &resource[offsets.start_offset..offsets.start_offset+offsets.end_offset];

        output_dir.create_dir_all(filepath.parent().ok_or(anyhow!("Invalid filename"))?)?;
        let mut f = output_dir.create(filepath)?;
        f.write_all(payload_data)?;
    }


    Ok(())
}
