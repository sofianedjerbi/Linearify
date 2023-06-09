use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::io::{BufReader, Cursor, Read, SeekFrom};

use byteorder::{BigEndian, ReadBytesExt};
use zstd::stream::Decoder;

const LINEAR_SIGNATURE: i64 = -4323716122432332390;
const LINEAR_VERSION: i8 = 2;
const LINEAR_SUPPORTED: [i8; 2] = [1, 2];
const HEADER_SIZE: i32 = 8192;

#[derive(Debug)]
pub struct Chunk {
    pub raw_chunk: Vec<u8>,
    pub x: usize,
    pub z: usize,
}

#[derive(Debug)]
pub struct Region {
    pub chunks: Vec<Option<Chunk>>,
    pub region_x: usize,
    pub region_z: usize,
    pub newest_timestamp: i32,
    pub timestamps: Vec<i32>,
}

impl Region {
    pub fn count_chunks(&self) -> usize {
        self.chunks.iter().filter(|&chunk| chunk.is_none()).count()
    }
}

pub fn open_linear(path: &str) -> Result<Region, Box<dyn Error>> {
    let coords: Vec<&str> = path.split('/').last().unwrap().split('.').collect();
    let region_x: usize = coords[1].parse::<usize>()?;
    let region_z: usize = coords[2].parse::<usize>()?;

    let file = File::open(path)?;
    let mut buffer = BufReader::new(file);

    // Go to the end 8 bytes before to read signature footer
    buffer.seek(SeekFrom::End(-8))?;
    let signature_footer = buffer.read_i64::<BigEndian>()?;
    buffer.seek(SeekFrom::Start(0))?; 
    let signature = buffer.read_i64::<BigEndian>()?;
    let version = buffer.read_i8()?;
    let newest_timestamp = buffer.read_i32::<BigEndian>()?;
    // Skip compression level (Byte): Unused
    buffer.seek(SeekFrom::Current(1))?;
    let chunk_count = buffer.read_i16::<BigEndian>()?;
    // Skip compressed_length level (Long): Unused
    buffer.seek(SeekFrom::Current(8))?;
    let compressed_length = buffer.read_i32::<BigEndian>()?;
    // Skip datahash (Long): Unused
    buffer.seek(SeekFrom::Current(8))?;

    if signature != LINEAR_SIGNATURE {
        return Err(format!("Invalid signature {}", signature).into());
    }
    if !LINEAR_SUPPORTED.iter().any(|&num| num == version) {
        return Err(format!("Invalid version {}", version).into());
    }
    if signature_footer != LINEAR_SIGNATURE {
        return Err(format!("Invalid footer signature {}", signature_footer).into());
    }

    let mut decoder = Decoder::with_buffer(buffer)?; // Decode with zstd
    let mut raw: Vec<u8> = Vec::new();
    decoder.read_to_end(&mut raw)?;
    let mut cursor = Cursor::new(&raw);

    let mut sizes: Vec<usize> = Vec::new();
    let mut timestamps: Vec<i32> = Vec::new();
    let mut chunks: Vec<Option<Chunk>> = Vec::new();

    let mut real_chunk_count = 0;
    let mut total_size = 0;
    for _ in 0..1024 {
        let size = cursor.read_i32::<BigEndian>()?;
        let timestamp = cursor.read_i32::<BigEndian>()?;
        total_size += size;
        real_chunk_count += (size != 0) as i16;
        sizes.push(size as usize);
        timestamps.push(timestamp);
    }

    if total_size + HEADER_SIZE != raw.len() as i32 {
        return Err("Invalid decompressed size".into());
    }

    if real_chunk_count != chunk_count {
        return Err("Invalid chunk count".into());
    }

    for i in 0..1024 {
        chunks[i] = match sizes[i] {
            0 => None,
            size => {
                let mut raw_chunk = vec![0u8; size];
                cursor.read_exact(&mut raw_chunk)?;
                Some(Chunk {
                    raw_chunk,
                    x: 32 * region_x + i % 32,
                    z: 32 * region_z + i / 32,
                })
            }
        };
    }

    Ok(Region {
        chunks,
        region_x,
        region_z,
        newest_timestamp,
        timestamps,
    })
}
