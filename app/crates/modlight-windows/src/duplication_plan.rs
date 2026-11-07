//! The plain logic of Desktop Duplication (`capture::duplication`): which output shows
//! which part of a region, how a mapped picture's rows reach a frame, when an output has
//! a picture, and when it is tried again.
//!
//! Nothing here touches the system. This is the one module of the crate that builds on
//! every system, so that its tests run on a Mac as well; it only uses what
//! `modlight-core` has.

use std::time::{Duration, Instant};

use modlight_core::capture::geometry::IRect;

/// How often an output that failed is tried again, at most.
pub(crate) const RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// How long an output whose picture cannot be read as it is (rotated, another format or
/// size) stays with GDI before it is looked at again. This hardly ever changes by itself,
/// and opening a duplication only to give it up is not free.
pub(crate) const UNSUITABLE_WAIT: Duration = Duration::from_secs(5);

/// How long a region that no output showed waits before the outputs are enumerated again,
/// in case they were out of reach only for a moment (a mode switch). A change of the
/// monitors plans everything again at once anyway.
pub(crate) const NO_OUTPUT_WAIT: Duration = Duration::from_secs(5);

/// How long a one-shot capture waits for a new picture.
pub(crate) const ONE_SHOT_WAIT: Duration = Duration::from_millis(200);

/// One output's share of a region. The three rectangles have the same size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    /// The output's place in the list given to [`split`].
    pub(crate) output: usize,
    /// The share on the desktop, physical pixels.
    pub(crate) desktop: IRect,
    /// The share in the output's own picture: the source box.
    pub(crate) source: IRect,
    /// The share in the frame.
    pub(crate) dest: IRect,
}

/// How a region lies on the outputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) pieces: Vec<Piece>,
    /// What no output shows, on the desktop: at most a few rectangles, none overlapping.
    pub(crate) rest: Vec<IRect>,
}

/// `part` without `hole`: the bands above and below the hole and what lies left and right
/// of it, at most four rectangles.
fn subtract(part: IRect, hole: IRect) -> Vec<IRect> {
    let hole = part.intersect(&hole);
    if !hole.has_area() {
        return vec![part];
    }
    [
        IRect::new(part.x, part.y, part.width, hole.y - part.y),
        IRect::new(
            part.x,
            hole.bottom(),
            part.width,
            part.bottom() - hole.bottom(),
        ),
        IRect::new(part.x, hole.y, hole.x - part.x, hole.height),
        IRect::new(
            hole.right(),
            hole.y,
            part.right() - hole.right(),
            hole.height,
        ),
    ]
    .into_iter()
    .filter(IRect::has_area)
    .collect()
}

/// A region (physical pixels) over `outputs`, their desktop rectangles in the same space.
/// Each output that overlaps the region gets its intersection; what lies on none of them
/// is the plan's rest. Displays that clone each other share a rectangle: the first shows it.
pub(crate) fn split(region: IRect, outputs: &[IRect]) -> Plan {
    let mut plan = Plan::default();
    if region.has_area() {
        plan.rest.push(region);
    }
    for (output, bounds) in outputs.iter().enumerate() {
        let desktop = region.intersect(bounds);
        let shown = plan
            .pieces
            .iter()
            .any(|piece| piece.desktop.contains_rect(&desktop));
        if !desktop.has_area() || shown {
            continue;
        }
        let size = (desktop.width, desktop.height);
        plan.pieces.push(Piece {
            output,
            desktop,
            source: IRect::new(desktop.x - bounds.x, desktop.y - bounds.y, size.0, size.1),
            dest: IRect::new(desktop.x - region.x, desktop.y - region.y, size.0, size.1),
        });
        plan.rest = plan
            .rest
            .iter()
            .flat_map(|part| subtract(*part, desktop))
            .collect();
    }
    plan
}

/// Some of a plan's parts lie off the virtual desktop `bounds` that GDI reads, so the frame
/// has to be made black before it is filled in (and not after: that would wipe out what
/// the outputs put there); the pointer drawn into the last frame goes with it. Given the
/// rest and the pieces, which GDI reads whenever their output cannot: outputs and monitors
/// disagree for a moment while a mode changes.
pub(crate) fn leaves_the_desktop<'a>(
    parts: impl IntoIterator<Item = &'a IRect>,
    bounds: IRect,
) -> bool {
    parts
        .into_iter()
        .any(|part| part.intersect(&bounds) != *part)
}

/// The bytes a mapped picture of `width` x `height` pixels takes with rows `pitch` bytes
/// apart: the last row needs no padding. `None` when a row does not fit its pitch.
pub(crate) fn mapped_length(pitch: usize, width: usize, height: usize) -> Option<usize> {
    let row = width * 4;
    (pitch >= row && height > 0).then(|| pitch * (height - 1) + row)
}

/// Copies a picture from rows `pitch` bytes apart into `dest` of `frame`, tightly packed
/// BGRA `frame_width` pixels wide. Refused, with nothing copied, when it does not fit.
pub(crate) fn copy_rows(
    source: &[u8],
    pitch: usize,
    frame: &mut [u8],
    frame_width: usize,
    dest: IRect,
) -> bool {
    if dest.x < 0 || dest.y < 0 || !dest.has_area() {
        return false;
    }
    let (x, y) = (dest.x as usize, dest.y as usize);
    let (width, height) = (dest.width as usize, dest.height as usize);
    let row = width * 4;
    let stride = frame_width * 4;
    let fits_frame = x + width <= frame_width && (y + height) * stride <= frame.len();
    let fits_source = mapped_length(pitch, width, height).is_some_and(|n| n <= source.len());
    if !fits_frame || !fits_source {
        return false;
    }
    for line in 0..height {
        let from = line * pitch;
        let to = (y + line) * stride + x * 4;
        frame[to..to + row].copy_from_slice(&source[from..from + row]);
    }
    true
}

/// `block` of a tightly packed BGRA picture is black. The fourth byte does not count: GDI
/// leaves it undefined. A block outside the picture is not black.
pub(crate) fn is_black(frame: &[u8], frame_width: usize, block: IRect) -> bool {
    if block.x < 0 || block.y < 0 || !block.has_area() {
        return false;
    }
    let (x, y) = (block.x as usize, block.y as usize);
    let (width, height) = (block.width as usize, block.height as usize);
    let stride = frame_width * 4;
    if x + width > frame_width || (y + height) * stride > frame.len() {
        return false;
    }
    (y..y + height).all(|line| {
        let start = line * stride + x * 4;
        frame[start..start + width * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[..3] == [0, 0, 0])
    })
}

/// Whether an output's staging texture holds a picture.
///
/// A `LastPresentTime` of zero means "the desktop image did not change", and a pointer
/// update alone has it too, so the first frames of a new duplication may hold no picture
/// at all. Only a frame with a present time counts. Until the first one, that part of a
/// recording comes from GDI: right for a still desktop, and a game presents all the time,
/// so it switches over at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Picture {
    held: bool,
}

impl Picture {
    /// An acquired frame: `copy` runs only when it has a present time, and only a finished
    /// copy makes a picture.
    pub(crate) fn offer<E>(
        &mut self,
        last_present_time: i64,
        copy: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), E> {
        if last_present_time != 0 {
            copy()?;
            self.held = true;
        }
        Ok(())
    }

    pub(crate) fn held(self) -> bool {
        self.held
    }
}

/// What is left of a one-shot capture's wait, in whole milliseconds; `None` when it is
/// over.
fn wait_left(started: Instant, now: Instant) -> Option<u32> {
    let left = ONE_SHOT_WAIT.checked_sub(now.saturating_duration_since(started))?;
    let left = left.as_millis() as u32;
    (left > 0).then_some(left)
}

/// A one-shot capture's wait for a picture, [`ONE_SHOT_WAIT`] at most. `acquire` takes one
/// frame, waiting no longer than the milliseconds it is given, and says whether there is
/// a picture now. `false` when the time ran out: the caller keeps what it has.
pub(crate) fn wait_for_picture<E>(
    mut now: impl FnMut() -> Instant,
    mut acquire: impl FnMut(u32) -> Result<bool, E>,
) -> Result<bool, E> {
    let started = now();
    while let Some(left) = wait_left(started, now()) {
        if acquire(left)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// When a part that was given up may be opened again: at once after a call failed (the
/// attempts themselves still come at most every [`RETRY_INTERVAL`]), after
/// [`UNSUITABLE_WAIT`] when its picture cannot be read as it is.
pub(crate) fn reopen_at(now: Instant, unsuitable: bool) -> Instant {
    if unsuitable {
        now + UNSUITABLE_WAIT
    } else {
        now
    }
}

/// When the outputs are to be enumerated again though no part asks for it, after a plan
/// of `pieces` parts: a region that no output showed is looked at again after
/// [`NO_OUTPUT_WAIT`]; parts on outputs ask by themselves when they fail.
pub(crate) fn replan_at(now: Instant, pieces: usize) -> Option<Instant> {
    (pieces == 0).then(|| now + NO_OUTPUT_WAIT)
}

/// Lets attempts through at most once per [`RETRY_INTERVAL`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Retry {
    last: Option<Instant>,
}

impl Retry {
    /// An attempt may be made at `now`; if so, it counts as made.
    pub(crate) fn due(&mut self, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= RETRY_INTERVAL);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// What a recording reads the screen through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Path {
    Duplication,
    /// Some of the parts through duplication, the others through GDI.
    Mixed {
        duplicated: usize,
        parts: usize,
    },
    Gdi,
}

/// The path of a recording of `parts` parts, `duplicated` of them through duplication.
pub(crate) fn path_of(duplicated: usize, parts: usize) -> Path {
    if duplicated == 0 {
        Path::Gdi
    } else if duplicated == parts {
        Path::Duplication
    } else {
        Path::Mixed { duplicated, parts }
    }
}

/// Says each path once, however often it is asked: the log gets the first path and each
/// change, never a line per frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PathLog {
    said: Option<Path>,
}

impl PathLog {
    /// `path` when it is news.
    pub(crate) fn update(&mut self, path: Path) -> Option<Path> {
        (self.said != Some(path)).then(|| {
            self.said = Some(path);
            path
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: IRect = IRect::new(-1920, 0, 1920, 1080);
    const MIDDLE: IRect = IRect::new(0, 0, 2560, 1440);
    const RIGHT: IRect = IRect::new(2560, 200, 1280, 1024);

    /// Every pixel of the region is in exactly one piece or one rest rectangle, and each
    /// piece's rectangles agree with each other and with its output.
    fn assert_exact_cover(region: IRect, outputs: &[IRect], plan: &Plan) {
        for piece in &plan.pieces {
            let output = outputs[piece.output];
            assert!(output.contains_rect(&piece.desktop), "{piece:?}");
            assert!(region.contains_rect(&piece.desktop), "{piece:?}");
            assert_eq!(piece.source.x, piece.desktop.x - output.x);
            assert_eq!(piece.source.y, piece.desktop.y - output.y);
            assert_eq!(piece.dest.x, piece.desktop.x - region.x);
            assert_eq!(piece.dest.y, piece.desktop.y - region.y);
            for rect in [piece.source, piece.dest] {
                assert_eq!(
                    (rect.width, rect.height),
                    (piece.desktop.width, piece.desktop.height)
                );
            }
        }
        for y in region.y..region.bottom() {
            for x in region.x..region.right() {
                let pieces = plan.pieces.iter().filter(|p| p.desktop.contains(x, y));
                let rest = plan.rest.iter().filter(|r| r.contains(x, y));
                assert_eq!(pieces.count() + rest.count(), 1, "pixel {x}, {y}");
            }
        }
    }

    #[test]
    fn a_region_on_one_output() {
        let region = IRect::new(100, 50, 640, 480);
        let plan = split(region, &[MIDDLE]);
        assert_eq!(
            plan.pieces,
            [Piece {
                output: 0,
                desktop: region,
                source: region,
                dest: IRect::new(0, 0, 640, 480),
            }]
        );
        assert!(plan.rest.is_empty());
    }

    #[test]
    fn a_whole_output() {
        let plan = split(MIDDLE, &[LEFT, MIDDLE, RIGHT]);
        assert_eq!(plan.pieces.len(), 1);
        assert_eq!(plan.pieces[0].output, 1);
        assert_eq!(plan.pieces[0].source, IRect::new(0, 0, 2560, 1440));
        assert!(plan.rest.is_empty());
    }

    #[test]
    fn a_region_across_monitors_at_negative_coordinates() {
        let region = IRect::new(-100, 10, 300, 40);
        let outputs = [LEFT, MIDDLE];
        let plan = split(region, &outputs);
        assert_eq!(plan.pieces.len(), 2);
        // The left monitor's part starts 100 pixels from its right edge.
        assert_eq!(plan.pieces[0].source, IRect::new(1820, 10, 100, 40));
        assert_eq!(plan.pieces[0].dest, IRect::new(0, 0, 100, 40));
        assert_eq!(plan.pieces[1].source, IRect::new(0, 10, 200, 40));
        assert_eq!(plan.pieces[1].dest, IRect::new(100, 0, 200, 40));
        assert!(plan.rest.is_empty());
        assert_exact_cover(region, &outputs, &plan);
    }

    #[test]
    fn monitors_of_different_heights_leave_a_gap() {
        // The right monitor starts 200 pixels down: its corner of the region is no one's.
        let region = IRect::new(2500, 150, 120, 100);
        let outputs = [MIDDLE, RIGHT];
        let plan = split(region, &outputs);
        assert_eq!(plan.pieces.len(), 2);
        assert_eq!(plan.pieces[1].source, IRect::new(0, 0, 60, 50));
        assert_eq!(plan.pieces[1].dest, IRect::new(60, 50, 60, 50));
        assert_eq!(plan.rest, [IRect::new(2560, 150, 60, 50)]);
        assert_exact_cover(region, &outputs, &plan);
    }

    #[test]
    fn partly_off_the_desktop() {
        let region = IRect::new(-10, -6, 30, 20);
        let plan = split(region, &[MIDDLE]);
        assert_eq!(plan.pieces[0].desktop, IRect::new(0, 0, 20, 14));
        assert_eq!(plan.pieces[0].source, IRect::new(0, 0, 20, 14));
        assert_eq!(plan.pieces[0].dest, IRect::new(10, 6, 20, 14));
        assert_eq!(plan.rest.len(), 2);
        assert_exact_cover(region, &[MIDDLE], &plan);
    }

    #[test]
    fn nothing_on_any_output() {
        let region = IRect::new(9000, 9000, 64, 32);
        let plan = split(region, &[LEFT, MIDDLE, RIGHT]);
        assert!(plan.pieces.is_empty());
        assert_eq!(plan.rest, [region]);
        // No outputs at all (a session without any) is the same: everything to GDI.
        assert_eq!(split(region, &[]).rest, [region]);
    }

    #[test]
    fn an_edge_touching_the_region_is_no_share() {
        let region = IRect::new(2560, 0, 100, 100);
        let plan = split(region, &[MIDDLE]);
        assert!(plan.pieces.is_empty());
        assert_eq!(plan.rest, [region]);
    }

    #[test]
    fn a_region_without_area_is_nothing() {
        assert_eq!(split(IRect::new(10, 10, 0, 40), &[MIDDLE]), Plan::default());
    }

    #[test]
    fn odd_sizes_and_offsets() {
        let region = IRect::new(1279, 717, 1283, 723);
        let outputs = [IRect::new(0, 0, 1281, 719), IRect::new(1281, 0, 1283, 721)];
        let plan = split(region, &outputs);
        assert_eq!(plan.pieces[0].desktop, IRect::new(1279, 717, 2, 2));
        assert_eq!(plan.pieces[1].source, IRect::new(0, 717, 1281, 4));
        assert_eq!(plan.pieces[1].dest, IRect::new(2, 0, 1281, 4));
        assert_exact_cover(region, &outputs, &plan);
    }

    #[test]
    fn cloned_displays_count_once() {
        let region = IRect::new(10, 10, 50, 50);
        let plan = split(region, &[MIDDLE, MIDDLE]);
        assert_eq!(plan.pieces.len(), 1);
        assert_eq!(plan.pieces[0].output, 0);
        assert!(plan.rest.is_empty());
    }

    #[test]
    fn many_layouts_cover_every_pixel_once() {
        // Four monitors around a hole, in small numbers so that each pixel can be counted.
        let outputs = [
            IRect::new(0, 0, 20, 10),
            IRect::new(20, 3, 7, 7),
            IRect::new(-13, 5, 13, 11),
            IRect::new(4, 10, 9, 5),
        ];
        for x in (-20..35).step_by(5) {
            for y in (-8..22).step_by(3) {
                for (width, height) in [(1, 1), (2, 2), (7, 3), (13, 9), (40, 30)] {
                    let region = IRect::new(x, y, width, height);
                    assert_exact_cover(region, &outputs, &split(region, &outputs));
                }
            }
        }
    }

    #[test]
    fn what_needs_a_black_background() {
        // The virtual desktop of LEFT, MIDDLE and RIGHT.
        let bounds = IRect::new(-1920, 0, 5760, 1440);
        assert_eq!(bounds, LEFT.union(&MIDDLE).union(&RIGHT));
        // A region on the outputs leaves nothing to GDI at all.
        assert!(!leaves_the_desktop(&[], bounds));
        let gap = IRect::new(2500, 150, 120, 100);
        let plan = split(gap, &[LEFT, MIDDLE, RIGHT]);
        // Between monitors of different heights, but inside the desktop: GDI reads it.
        assert_eq!(plan.rest, [IRect::new(2560, 150, 60, 50)]);
        assert!(!leaves_the_desktop(&plan.rest, bounds));
        let pieces = plan.pieces.iter().map(|piece| &piece.desktop);
        assert!(!leaves_the_desktop(plan.rest.iter().chain(pieces), bounds));
        // Partly and wholly off it.
        assert!(leaves_the_desktop(&[IRect::new(-1930, 0, 20, 20)], bounds));
        assert!(leaves_the_desktop(&[IRect::new(9000, 0, 20, 20)], bounds));
        assert!(leaves_the_desktop(
            &[IRect::new(0, 0, 8, 8), IRect::new(0, 1440, 8, 8)],
            bounds
        ));
        // No monitors at all.
        assert!(leaves_the_desktop(
            &[IRect::new(0, 0, 8, 8)],
            IRect::default()
        ));
    }

    #[test]
    fn a_piece_beyond_the_monitors_needs_black_too() {
        // While a mode changes, an output may still be larger than its monitor: GDI cannot
        // read all of that piece when its duplication fails, so the frame is made black.
        let bounds = IRect::new(0, 0, 1920, 1080);
        let plan = split(IRect::new(1800, 1000, 200, 100), &[MIDDLE]);
        assert!(plan.rest.is_empty());
        assert!(!leaves_the_desktop(&plan.rest, bounds));
        let pieces = plan.pieces.iter().map(|piece| &piece.desktop);
        assert!(leaves_the_desktop(plan.rest.iter().chain(pieces), bounds));
    }

    #[test]
    fn all_gdi_is_the_recording_as_it_always_was() {
        // Without outputs the rest is the region, and the two tests of the old loop
        // (`part != rect` to fill, `part` to copy) come out the same.
        let bounds = IRect::new(0, 0, 1920, 1080);
        for region in [
            IRect::new(10, 10, 64, 32),
            IRect::new(-10, 10, 64, 32),
            IRect::new(1900, 1070, 64, 32),
            IRect::new(4000, 10, 64, 32),
        ] {
            let plan = split(region, &[]);
            assert_eq!(plan.rest, [region]);
            let part = region.intersect(&bounds);
            assert_eq!(leaves_the_desktop(&plan.rest, bounds), part != region);
            assert_eq!(plan.rest[0].intersect(&bounds), part);
        }
    }

    /// A picture whose every byte tells where it is, and a pitch with padding of 0xEE.
    fn padded_picture(width: usize, height: usize, pitch: usize) -> Vec<u8> {
        let mut picture = vec![0xEE; pitch * height];
        for y in 0..height {
            for x in 0..width * 4 {
                picture[y * pitch + x] = (y * 16 + x) as u8;
            }
        }
        picture
    }

    #[test]
    fn rows_with_a_pitch_larger_than_the_row() {
        let (width, height, pitch) = (3, 2, 32);
        let source = padded_picture(width, height, pitch);
        let frame_width = 5;
        let mut frame = vec![7u8; frame_width * 4 * 4];
        let dest = IRect::new(1, 1, width as i32, height as i32);
        assert!(copy_rows(&source, pitch, &mut frame, frame_width, dest));
        for y in 0..4 {
            for x in 0..frame_width * 4 {
                let inside = (1..3).contains(&y) && (4..16).contains(&x);
                let want = if inside {
                    ((y - 1) * 16 + (x - 4)) as u8
                } else {
                    7
                };
                assert_eq!(frame[y * frame_width * 4 + x], want, "byte {x} of row {y}");
            }
        }
        // None of the padding came along.
        assert!(!frame.contains(&0xEE));
    }

    #[test]
    fn rows_with_a_pitch_equal_to_the_row() {
        let source: Vec<u8> = (0..32).collect();
        let mut frame = vec![0u8; 32];
        assert!(copy_rows(
            &source,
            16,
            &mut frame,
            4,
            IRect::new(0, 0, 4, 2)
        ));
        assert_eq!(frame, source);
    }

    #[test]
    fn an_odd_width_ending_at_the_mapped_memory() {
        // The last row of a mapping need not have its padding: reading it must stop at
        // the row's end, as the length handed out by `mapped_length`.
        let (width, height, pitch) = (3, 3, 64);
        let length = mapped_length(pitch, width, height).unwrap();
        assert_eq!(length, 64 * 2 + 12);
        let source = padded_picture(width, height, pitch);
        let mut frame = vec![0u8; width * height * 4];
        let dest = IRect::new(0, 0, 3, 3);
        assert!(copy_rows(&source[..length], pitch, &mut frame, width, dest));
        assert_eq!(frame[frame.len() - 1], (2 * 16 + 11) as u8);
    }

    #[test]
    fn rows_that_do_not_fit_are_refused_whole() {
        let source = padded_picture(4, 4, 16);
        let mut frame = vec![9u8; 4 * 4 * 4];
        let whole = IRect::new(0, 0, 4, 4);
        // A pitch smaller than a row, a source too short.
        assert!(!copy_rows(&source, 15, &mut frame, 4, whole));
        assert!(!copy_rows(&source[..63], 16, &mut frame, 4, whole));
        // Past the frame's right and bottom edges, before its top left, nothing at all.
        assert!(!copy_rows(
            &source,
            16,
            &mut frame,
            4,
            IRect::new(1, 0, 4, 1)
        ));
        assert!(!copy_rows(
            &source,
            16,
            &mut frame,
            4,
            IRect::new(0, 1, 1, 4)
        ));
        assert!(!copy_rows(
            &source,
            16,
            &mut frame,
            4,
            IRect::new(-1, 0, 2, 2)
        ));
        assert!(!copy_rows(
            &source,
            16,
            &mut frame,
            4,
            IRect::new(0, -1, 2, 2)
        ));
        assert!(!copy_rows(
            &source,
            16,
            &mut frame,
            4,
            IRect::new(0, 0, 0, 2)
        ));
        assert!(frame.iter().all(|byte| *byte == 9));
    }

    #[test]
    fn the_length_of_a_mapping() {
        assert_eq!(mapped_length(16, 4, 1), Some(16));
        assert_eq!(mapped_length(64, 4, 2), Some(80));
        assert_eq!(mapped_length(15, 4, 2), None);
        assert_eq!(mapped_length(16, 4, 0), None);
    }

    #[test]
    fn black_is_about_the_colours_only() {
        let width = 4;
        // Black with GDI's undefined fourth byte in it.
        let mut frame = vec![0u8; width * 3 * 4];
        for pixel in frame.as_chunks_mut::<4>().0 {
            pixel[3] = 0xAB;
        }
        let whole = IRect::new(0, 0, 4, 3);
        assert!(is_black(&frame, width, whole));
        // One channel of one pixel is enough, each of the three.
        for channel in 0..3 {
            let mut lit = frame.clone();
            lit[(2 * width + 3) * 4 + channel] = 1;
            assert!(!is_black(&lit, width, whole), "channel {channel}");
            // A block beside that pixel stays black.
            assert!(is_black(&lit, width, IRect::new(0, 0, 3, 3)));
            assert!(is_black(&lit, width, IRect::new(0, 0, 4, 2)));
            assert!(!is_black(&lit, width, IRect::new(3, 2, 1, 1)));
        }
    }

    #[test]
    fn a_block_outside_the_picture_is_not_black() {
        let frame = vec![0u8; 4 * 4 * 4];
        assert!(!is_black(&frame, 4, IRect::new(1, 0, 4, 1)));
        assert!(!is_black(&frame, 4, IRect::new(0, 1, 1, 4)));
        assert!(!is_black(&frame, 4, IRect::new(-1, 0, 2, 2)));
        assert!(!is_black(&frame, 4, IRect::new(0, 0, 0, 0)));
    }

    /// Offers a frame to `picture` and says whether the copy ran.
    fn offered(picture: &mut Picture, last_present_time: i64) -> bool {
        let mut copied = false;
        let result: Result<(), ()> = picture.offer(last_present_time, || {
            copied = true;
            Ok(())
        });
        assert!(result.is_ok());
        copied
    }

    #[test]
    fn a_frame_without_a_present_time_is_no_picture() {
        let mut picture = Picture::default();
        // A new duplication has nothing: that part comes from GDI.
        assert!(!picture.held());
        // The first frames, or the pointer alone: not copied, however many of them come.
        for _ in 0..3 {
            assert!(!offered(&mut picture, 0));
            assert!(!picture.held());
        }
        // The first one with a present time is.
        assert!(offered(&mut picture, 12345));
        assert!(picture.held());
    }

    #[test]
    fn a_picture_stays_through_frames_without_a_present_time() {
        let mut picture = Picture::default();
        assert!(offered(&mut picture, 1));
        // The pointer moved: the staging texture keeps the last picture, and is not
        // written over with a frame that may hold nothing.
        assert!(!offered(&mut picture, 0));
        assert!(picture.held());
        // A new image is copied again.
        assert!(offered(&mut picture, 2));
        assert!(picture.held());
    }

    #[test]
    fn present_times_are_counter_values() {
        for time in [1, -1, i64::MAX, i64::MIN] {
            assert!(offered(&mut Picture::default(), time), "{time}");
        }
    }

    #[test]
    fn only_a_finished_copy_makes_a_picture() {
        let mut picture = Picture::default();
        assert_eq!(picture.offer(7, || Err("no copy")), Err("no copy"));
        assert!(!picture.held());
    }

    /// `wait_for_picture` on a clock of its own: each `acquire` call uses up the time
    /// `step` says, from the milliseconds it was given.
    fn waited(
        mut step: impl FnMut(u32) -> (u64, Result<bool, &'static str>),
    ) -> (u32, Result<bool, &'static str>) {
        let start = Instant::now();
        let clock = std::cell::Cell::new(start);
        let mut calls = 0;
        let result = wait_for_picture(
            || clock.get(),
            |left| {
                calls += 1;
                assert!(left > 0 && left <= 200, "{left}");
                let (used, answer) = step(left);
                clock.set(clock.get() + Duration::from_millis(used));
                answer
            },
        );
        (calls, result)
    }

    #[test]
    fn a_one_shot_capture_takes_the_first_frame_with_a_present_time() {
        let mut picture = Picture::default();
        let mut times = [0, 0, 55].into_iter();
        let (calls, result) = waited(|_| {
            let time = times.next().unwrap();
            let offered: Result<(), &str> = picture.offer(time, || Ok(()));
            (5, offered.map(|()| picture.held()))
        });
        assert_eq!(result, Ok(true));
        assert_eq!(calls, 3);
    }

    #[test]
    fn a_one_shot_capture_gives_up_on_frames_without_a_present_time() {
        let mut picture = Picture::default();
        let (calls, result) = waited(|_| {
            let offered: Result<(), &str> = picture.offer(0, || panic!("copied"));
            (30, offered.map(|()| picture.held()))
        });
        // 200 ms at 30 ms a frame: the caller keeps the GDI picture.
        assert_eq!(result, Ok(false));
        assert_eq!(calls, 7);
    }

    #[test]
    fn a_one_shot_capture_stops_at_a_timeout_of_the_whole_wait() {
        // Nothing arrives: `AcquireNextFrame` uses up what it was given and times out.
        let (calls, result) = waited(|left| (left as u64, Ok(false)));
        assert_eq!(result, Ok(false));
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_one_shot_capture_stops_at_a_failure() {
        let (calls, result) = waited(|_| (1, Err("access lost")));
        assert_eq!(result, Err("access lost"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn the_wait_left_of_a_one_shot_capture() {
        let started = Instant::now();
        let at = |ms| started + Duration::from_millis(ms);
        assert_eq!(wait_left(started, started), Some(200));
        assert_eq!(wait_left(started, at(150)), Some(50));
        assert_eq!(wait_left(started, at(199)), Some(1));
        assert_eq!(wait_left(started, at(200)), None);
        assert_eq!(wait_left(started, at(5000)), None);
        // Less than a millisecond left is over too: a wait of 0 would not wait.
        assert_eq!(
            wait_left(started, at(199) + Duration::from_micros(500)),
            None
        );
        // A clock read before the start does not stretch the wait.
        assert_eq!(wait_left(at(10), started), Some(200));
    }

    #[test]
    fn attempts_come_at_most_every_500_ms() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut retry = Retry::default();
        // The first attempt goes at once.
        assert!(retry.due(start));
        assert!(!retry.due(start));
        assert!(!retry.due(at(1)));
        assert!(!retry.due(at(499)));
        assert!(retry.due(at(500)));
        // Counted from the last attempt, not from the first one.
        assert!(!retry.due(at(999)));
        assert!(retry.due(at(1000)));
        // A refused attempt does not push the next one back.
        assert!(!retry.due(at(1499)));
        assert!(retry.due(at(1500)));
        // After a long quiet time an attempt goes at once again.
        assert!(retry.due(at(60_000)));
        assert!(!retry.due(at(60_001)));
    }

    #[test]
    fn the_retry_waits_are_the_documented_ones() {
        assert_eq!(RETRY_INTERVAL, Duration::from_millis(500));
        assert_eq!(UNSUITABLE_WAIT, Duration::from_secs(5));
        assert_eq!(NO_OUTPUT_WAIT, Duration::from_secs(5));
        assert_eq!(ONE_SHOT_WAIT, Duration::from_millis(200));
    }

    #[test]
    fn a_failed_call_is_tried_again_at_the_next_attempt() {
        let now = Instant::now();
        assert_eq!(reopen_at(now, false), now);
        let mut retry = Retry::default();
        assert!(retry.due(now));
        // Due at once, but the attempt waits for its 500 ms.
        let failed = now + Duration::from_millis(20);
        let reopen = reopen_at(failed, false);
        assert!(failed >= reopen);
        assert!(!retry.due(failed));
        assert!(retry.due(now + RETRY_INTERVAL));
    }

    #[test]
    fn an_unsuitable_output_waits_five_seconds() {
        let now = Instant::now();
        let reopen = reopen_at(now, true);
        assert_eq!(reopen, now + Duration::from_secs(5));
        // Attempts for other outputs meanwhile do not open it.
        assert!(now + Duration::from_millis(4999) < reopen);
    }

    #[test]
    fn a_region_on_no_output_is_planned_again_now_and_then() {
        let now = Instant::now();
        assert_eq!(replan_at(now, 0), Some(now + Duration::from_secs(5)));
        // Parts on outputs ask by themselves.
        assert_eq!(replan_at(now, 1), None);
        assert_eq!(replan_at(now, 3), None);
    }

    #[test]
    fn the_path_of_a_recording() {
        assert_eq!(path_of(0, 0), Path::Gdi);
        assert_eq!(path_of(0, 2), Path::Gdi);
        assert_eq!(path_of(2, 2), Path::Duplication);
        assert_eq!(
            path_of(1, 3),
            Path::Mixed {
                duplicated: 1,
                parts: 3
            }
        );
    }

    #[test]
    fn each_path_is_said_once() {
        let mut log = PathLog::default();
        assert_eq!(log.update(Path::Gdi), Some(Path::Gdi));
        // A frame after frame of the same says nothing.
        for _ in 0..1000 {
            assert_eq!(log.update(Path::Gdi), None);
        }
        assert_eq!(log.update(Path::Duplication), Some(Path::Duplication));
        assert_eq!(log.update(Path::Duplication), None);
        let mixed = Path::Mixed {
            duplicated: 1,
            parts: 2,
        };
        assert_eq!(log.update(mixed), Some(mixed));
        // Another share is another path.
        let other = Path::Mixed {
            duplicated: 2,
            parts: 3,
        };
        assert_eq!(log.update(other), Some(other));
        // Back to one said before: a change is a change.
        assert_eq!(log.update(Path::Gdi), Some(Path::Gdi));
    }
}
