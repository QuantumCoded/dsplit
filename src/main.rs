use clap::{App, Arg};
use grid::Grid;
use image::{buffer::RowsMut, ImageBuffer, Luma, Pixel, Rgb};
use imageproc::filter::Kernel;
use lab::Lab;
use rgb::AsPixels;
use std::{io, ops::Deref};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use std::{
    ops::DerefMut,
    process::{Command, ExitStatus, Stdio},
};


#[derive(Clone, Copy, Debug)]
enum Direction {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug)]
struct Line {
    dir: Direction,
    x: usize,
    y: usize,
    len: usize,
}

struct GridPair(Grid<f32>, Grid<f32>);

impl GridPair {
    fn filter(&mut self, f: impl FnMut(&mut f32) + Copy) {
        self.0.iter_mut().for_each(f);
        self.1.iter_mut().for_each(f);
    }

    fn lines(&self) -> Vec<Line> {
        let mut lines = vec![];

        for col in 0..self.0.cols() {
            let mut start = None;
            let col_iter = self.0.iter_col(col);

            for (idx, val) in col_iter.enumerate() {
                match start {
                    None if *val >= 1. => start = Some(idx), // start the line
                    Some(start_idx) if *val < 1. => {
                        // end the line
                        lines.push(Line {
                            dir: Direction::Vertical,
                            x: col,
                            y: start_idx,
                            len: idx - start_idx,
                        });

                        start = None;
                    }
                    _ => {}
                }
            }
        }

        for row in 0..self.1.rows() {
            let mut start = None;
            let row_iter = self.1.iter_row(row);

            for (idx, val) in row_iter.enumerate() {
                match start {
                    None if *val >= 1. => start = Some(idx), // start the line
                    Some(start_idx) if *val < 1. => {
                        // end the line
                        lines.push(Line {
                            dir: Direction::Horizontal,
                            x: start_idx,
                            y: row,
                            len: idx - start_idx,
                        });

                        start = None;
                    }
                    _ => {}
                }
            }
        }

        lines
    }
}

trait Lines {
    fn discard_shorter_than(&mut self, size: usize);
    fn to_image(&self, width: usize, height: usize) -> ImageBuffer<Rgb<u8>, Vec<u8>>;
}

// impl for a generic deref to [Line]
impl Lines for Vec<Line> {
    // fn get_shorter_than, non-mutable!
    fn discard_shorter_than(&mut self, size: usize) {
        *self = self
            .iter()
            .filter(|line| line.len >= size)
            .map(|l| *l)
            .collect::<Vec<Line>>();

        // when disregarding short lines they should be blotted out of the grids shouldn't they?
        // so maybe this lines thing actually needs to hold a GridPair?

        // future me: not if we don't use the diff grids again, see those just store the differences
        // once we have the lines all the representation is done with those so we don't acutally
        // need the diff grids anymore unless we're doing a different transform or something, idk
    }
    fn to_image(&self, width: usize, height: usize) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
        let mut img_grid: Grid<[u8; 3]> = Grid::new(height, width);

        for line in self {
            match line.dir {
                Direction::Horizontal => img_grid
                    .iter_row_mut(line.y)
                    .skip(line.x)
                    .take(line.len)
                    .for_each(|p| *p = [255, p[1], 0]),
                Direction::Vertical => img_grid
                    .iter_col_mut(line.x)
                    .skip(line.y)
                    .take(line.len)
                    .for_each(|p| *p = [p[0], 255, 0]),
            }
        }

        let buf: Vec<u8> = img_grid.iter().flatten().map(|v| *v).collect();
        image::ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width as u32, height as u32, buf).unwrap()
    }
}

/// Checks that a version of ffmpeg is accesable
fn ffmpeg_present() -> bool {
    if let Ok(status) = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .status()
    {
        status.success()
    } else {
        false
    }
}

/// Use ffmpeg to split a video into PNG images
fn create_image_sequence(
    video: impl AsRef<Path>,
    scale: f64,
    output: impl AsRef<Path>,
) -> io::Result<ExitStatus> {
    println!("creating image sequence from {:?}", video.as_ref());

    Command::new("ffmpeg")
        .args(&["-loglevel", "quiet", "-stats"])
        .arg("-i")
        .arg(video.as_ref())
        .arg("-vf")
        .arg(format!("scale=iw*{0}:ih*{0}", scale))
        .arg(output.as_ref().join("%00d.png"))
        .status()
}

fn main() {
    // check that ffmpeg exists (before trying to use it)
    if !ffmpeg_present() {
        todo!("add code to get ffmpeg");
    } else {
        println!("found ffmpeg!");
    }

    let matches = App::new("dsplit")
        .arg(
            Arg::with_name("INPUT")
                .help("The input file")
                .required(true),
        )
        .arg(
            Arg::with_name("scale")
                .short("s")
                .long("scale")
                .default_value(".1")
                .help("The scale factor for the size of each video frame (smaller = faster)"),
        )
        .get_matches();

    let scale: f64 = matches
        .value_of("scale")
        .unwrap()
        .parse()
        .expect("failed to parse scale value");
    let input = matches.value_of("INPUT").unwrap();

    // determine if input is an image or video

    let img = image::open(input).unwrap().to_rgb8();
    let mut diff_grid_x: Grid<f32> = Grid::new(img.height() as usize, img.width() as usize - 1);
    let mut diff_grid_y: Grid<f32> = Grid::new(img.height() as usize - 1, img.width() as usize);
    let img_grid = Grid::from_vec(
        img.pixels().map(|p| p.channels()).collect(),
        img.width() as usize,
    );

    // get edges across columns
    for row in 0..img_grid.rows() {
        let mut diff_row = diff_grid_x.iter_row_mut(row);
        let mut img_row = img_grid.iter_row(row);

        let rgb = img_row.next().expect("image is empty");
        let mut prev_lab = Lab::from_rgb(&[rgb[0], rgb[1], rgb[2]]);
        while let Some(rgb) = img_row.next() {
            let curr_lab = Lab::from_rgb(&[rgb[0], rgb[1], rgb[2]]);
            *diff_row.next().unwrap() = prev_lab.squared_distance(&curr_lab) / 10000.;
            prev_lab = curr_lab;
        }
    }

    // get edges across rows
    for col in 0..img_grid.cols() {
        let mut diff_col = diff_grid_y.iter_col_mut(col);
        let mut img_col = img_grid.iter_col(col);

        let rgb = img_col.next().expect("image is empty");
        let mut prev_lab = Lab::from_rgb(&[rgb[0], rgb[1], rgb[2]]);
        while let Some(rgb) = img_col.next() {
            let curr_lab = Lab::from_rgb(&[rgb[0], rgb[1], rgb[2]]);
            *diff_col.next().unwrap() = prev_lab.squared_distance(&curr_lab) / 10000.;
            prev_lab = curr_lab;
        }
    }

    let mut grid_pair = GridPair(diff_grid_x, diff_grid_y);
    grid_pair.filter(|value| *value = if *value > 0.0025 /* MAGIC */ { 1. } else { 0. });

    let mut lines = grid_pair.lines();
    lines.discard_shorter_than(20  /* MAGIC */);

    println!("{:?}", lines);

    lines
        .to_image(img.width() as usize, img.height() as usize)
        .save("edges.png")
        .unwrap();

    // might need to keep the original GridPair for later, if so, derive Clone and clone here

/*     image::ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(
        img.width() - 1,
        img.height(),
        grid_pair.0.iter().map(|s| (*s * 255.) as u8).collect(),
    )
    .unwrap()
    .save("edge_x.png")
    .unwrap();

    image::ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(
        img.width(),
        img.height() - 1,
        grid_pair.1.iter().map(|s| (*s * 255.) as u8).collect(),
    )
    .unwrap()
    .save("edge_y.png")
    .unwrap(); */
}
