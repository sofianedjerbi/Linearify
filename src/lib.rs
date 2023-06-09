use std::error::Error;
use std::fmt;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::prelude::*;
use std::io::{BufReader, BufWriter, Cursor, Read, SeekFrom};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use zstd::stream::decode_all;
use zstd::stream::encode_all;

const LINEAR_SIGNATURE: i64 = -4323716122432332390;
const LINEAR_VERSION: i8 = 2;
const LINEAR_SUPPORTED: [i8; 2] = [1, 2];
const HEADER_SIZE: i32 = 8192;

#[derive(Clone)]
pub struct Chunk {
    pub raw_chunk: Vec<u8>,
    pub x: usize,
    pub z: usize,
}

// Don't print raw_chunk
impl fmt::Debug for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Chunk {{ x: {}, z: {} }}",
            self.x, self.z
        )
    }
}

#[derive(Clone, Debug)]
pub struct Region {
    pub chunks: Vec<Option<Chunk>>,
    pub region_x: usize,
    pub region_z: usize,
    pub timestamps: Vec<i32>,
    pub newest_timestamp: i64,
}

impl fmt::Display for Region {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, chunk) in self.chunks.iter().enumerate() {
            match chunk {
                Some(_) => print!("■"),
                None => print!("□"),
            }
            if index % 32 == 31 {
                println!();
            }
        }
        Ok(())
    }
}

impl Region {
    fn count_chunks(&self) -> i16 {
        self.chunks.iter().filter(|&chunk| chunk.is_some()).count() as i16
    }

    pub fn write_linear(&self, dir: &str, compression_level: i32) -> Result<(), Box<dyn Error>> {
        let path = format!("{}/r.{}.{}.linear", dir, self.region_x, self.region_z);
        let wip_path = format!("{}/r.{}.{}.linear.wip", dir, self.region_x, self.region_z);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&wip_path)?;

        // Get chunks data
        let mut raw_data: Vec<u8> = Vec::new();
        for i in 0..1024 {
            if let Some(chunk) = &self.chunks[i] {
                let size: i32 = chunk.raw_chunk.len() as i32;
                let timestamp: i32 = self.timestamps[i];
                raw_data.extend_from_slice(&size.to_be_bytes());
                raw_data.extend_from_slice(&timestamp.to_be_bytes());
            } else {
                raw_data.extend_from_slice(&0u32.to_be_bytes()); // Write size 0 for empty chunks
                raw_data.extend_from_slice(&0u32.to_be_bytes()); // Write timestamp 0 for empty chunks
            }
        }

        for i in 0..1024 {
            if let Some(chunk) = &self.chunks[i] {
                raw_data.extend_from_slice(chunk.raw_chunk.as_slice());
            }
        }
        let raw_cursor = Cursor::new(&raw_data);
        let encoded: Vec<u8> = encode_all(raw_cursor, compression_level)?; // Encode it

        // Write file
        let mut buffer = BufWriter::new(file);
        // Header
        let chunk_count: i16 = self.count_chunks();
        buffer.write_i64::<BigEndian>(LINEAR_SIGNATURE)?; // Superblock
        buffer.write_i8(LINEAR_VERSION)?; // Version
        buffer.write_i64::<BigEndian>(self.newest_timestamp)?; // Newest timestamp
        buffer.write_i8(compression_level as i8)?; // Compression level
        buffer.write_i16::<BigEndian>(chunk_count)?; // Chunk count
        buffer.write_i32::<BigEndian>(encoded.len() as i32)?; // Compressed size
        buffer.write_i64::<BigEndian>(0)?; // Datahash: skip, unimplemented

        // Chunk data
        buffer.write_all(encoded.as_slice())?;

        // Write signature footer
        buffer.write_i64::<BigEndian>(LINEAR_SIGNATURE)?;

        // Flush & move
        buffer.flush()?;
        fs::rename(wip_path, path)?;
        Ok(())
    }
}

pub fn open_linear(path: &str) -> Result<Region, Box<dyn Error>> {
    let coords: Vec<&str> = path.split('/').last().unwrap().split('.').collect();
    let region_x: usize = coords[1].parse::<usize>()?;
    let region_z: usize = coords[2].parse::<usize>()?;

    let file = File::open(path)?;
    let mut buffer = BufReader::new(file);

    // Read chunk data
    // Go to the end 8 bytes before to read signature footer
    buffer.seek(SeekFrom::End(-8))?;
    let signature_footer = buffer.read_i64::<BigEndian>()?;
    buffer.seek(SeekFrom::Start(0))?; 
    let signature = buffer.read_i64::<BigEndian>()?;
    let version = buffer.read_i8()?;
    let newest_timestamp = buffer.read_i64::<BigEndian>()?;
    // Skip compression level (Byte): Unused
    buffer.seek(SeekFrom::Current(1))?;
    let chunk_count = buffer.read_i16::<BigEndian>()?;
    let compressed_length = buffer.read_i32::<BigEndian>()?;
    // Skip datahash (Long): Unused
    buffer.seek(SeekFrom::Current(8))?;

    // Verify data
    if signature != LINEAR_SIGNATURE {
        return Err(format!("Invalid signature: {}", signature).into());
    }
    if !LINEAR_SUPPORTED.iter().any(|&num| num == version) {
        return Err(format!("Invalid version: {}", version).into());
    }
    if signature_footer != LINEAR_SIGNATURE {
        return Err(format!("Invalid footer signature: {}", signature_footer).into());
    }

    // Read raw chunk
    let mut raw = vec![0u8; compressed_length as usize];
    buffer.read_exact(&mut raw)?;
    let raw_cursor = Cursor::new(&raw);
    // Decode data
    let decoded: Vec<u8> = decode_all(raw_cursor)?;
    let mut cursor = Cursor::new(&decoded);

    // Start deserializing
    let mut sizes: Vec<usize> = Vec::new();
    let mut timestamps: Vec<i32> = Vec::new();
    let mut chunks: Vec<Option<Chunk>> = vec![None; 1024];

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

    // Check if chunk data is corrupted
    if total_size + HEADER_SIZE != decoded.len() as i32 {
        return Err("Invalid decompressed size: {}".into());
    }

    if real_chunk_count != chunk_count {
        return Err(format!("Invalid chunk count {}/{}", chunk_count, real_chunk_count).into());
    }

    // Save raw chunk data
    for i in 0..1024 {
        if sizes[i] > 0 {
            let mut raw_chunk = vec![0u8; sizes[i]];
            cursor.read_exact(&mut raw_chunk)?;
            chunks[i] = Some(Chunk {
                raw_chunk,
                x: 32 * region_x + i % 32,
                z: 32 * region_z + i / 32,
            });
        }
    }

    Ok(Region {
        chunks,
        region_x,
        region_z,
        timestamps,
        newest_timestamp,
    })
}
