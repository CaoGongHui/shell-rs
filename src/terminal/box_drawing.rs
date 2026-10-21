//! Box drawing and block elements, drawn to the edges of their cells rather
//! than taken from the font. A font's `│` is only as tall as its letters, so
//! at the line heights terminals use a column of them comes apart, as tmux's
//! pane borders did. The diagonals still come from the font.

use gpui_kit::*;

/// A cell's size and the width of a light line, in device pixels: every line
/// then falls on whole pixels and meets the next cell's.
#[derive(Clone, Copy, Debug)]
pub(super) struct CellMetrics {
    width: f32,
    height: f32,
    light: f32,
}

impl CellMetrics {
    pub(super) fn new(
        cell_width: Pixels,
        line_height: Pixels,
        font_size: Pixels,
        scale: f32,
    ) -> Self {
        Self {
            width: (cell_width.as_f32() * scale).round(),
            height: (line_height.as_f32() * scale).round(),
            // About a letter's stroke: one pixel at 13 px, two at 150 %.
            light: (font_size.as_f32() * scale / 12.).round().max(1.),
        }
    }
}

/// What draws a character, in device pixels from its cell's top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Shape {
    /// Filled with the text's color, `alpha` of it.
    Fill { rect: Bounds<f32>, alpha: f32 },
    /// A line `thickness` wide bending at `bend` of `rect`, its border there
    /// rounded. `rect` is twice the bend's size, past the cell, which clips
    /// it.
    Arc {
        rect: Bounds<f32>,
        bend: Bend,
        thickness: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Bend {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

/// How `character` is drawn in a cell, or `None` if the font draws it.
pub(super) fn shapes(character: char, cell: CellMetrics) -> Option<Vec<Shape>> {
    match character as u32 {
        0x256D..=0x2570 => Some(vec![arc(character, cell)]),
        0x2500..=0x257F => lines(character, cell),
        0x2580..=0x259F => Some(blocks(character, cell)),
        _ => None,
    }
}

/// The quads that draw `shapes` in `color` in the cell at `origin`.
pub(super) fn quads(
    shapes: &[Shape],
    origin: Point<Pixels>,
    scale: f32,
    color: Hsla,
) -> impl Iterator<Item = PaintQuad> {
    let bounds = move |rect: Bounds<f32>| {
        Bounds::new(
            point(
                origin.x + px(rect.origin.x / scale),
                origin.y + px(rect.origin.y / scale),
            ),
            size(px(rect.size.width / scale), px(rect.size.height / scale)),
        )
    };
    shapes.iter().map(move |shape| match *shape {
        Shape::Fill { rect, alpha } => fill(bounds(rect), color.opacity(alpha)),
        Shape::Arc {
            rect,
            bend,
            thickness,
        } => {
            let radius = px(rect.size.width.min(rect.size.height) / 2. / scale);
            let line = px(thickness / scale);
            let mut corners = Corners::default();
            let mut edges = Edges::default();
            match bend {
                Bend::TopLeft => (corners.top_left, edges.top, edges.left) = (radius, line, line),
                Bend::TopRight => {
                    (corners.top_right, edges.top, edges.right) = (radius, line, line)
                }
                Bend::BottomRight => {
                    (corners.bottom_right, edges.bottom, edges.right) = (radius, line, line)
                }
                Bend::BottomLeft => {
                    (corners.bottom_left, edges.bottom, edges.left) = (radius, line, line)
                }
            }
            quad(
                bounds(rect),
                corners,
                transparent_black(),
                edges,
                color,
                BorderStyle::Solid,
            )
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Weight {
    Light,
    Heavy,
    Double,
}

/// U+2500 to U+257F: the arms up, right, down and left of each, light,
/// heavy, double or none (`.`). Dashed lines are their solid ones here; the
/// arcs and diagonals are drawn otherwise.
#[rustfmt::skip]
const LINES: [&str; 0x80] = [
    ".l.l", ".h.h", "l.l.", "h.h.", ".l.l", ".h.h", "l.l.", "h.h.", // ─━│┃┄┅┆┇
    ".l.l", ".h.h", "l.l.", "h.h.", ".ll.", ".hl.", ".lh.", ".hh.", // ┈┉┊┋┌┍┎┏
    "..ll", "..lh", "..hl", "..hh", "ll..", "lh..", "hl..", "hh..", // ┐┑┒┓└┕┖┗
    "l..l", "l..h", "h..l", "h..h", "lll.", "lhl.", "hll.", "llh.", // ┘┙┚┛├┝┞┟
    "hlh.", "hhl.", "lhh.", "hhh.", "l.ll", "l.lh", "h.ll", "l.hl", // ┠┡┢┣┤┥┦┧
    "h.hl", "h.lh", "l.hh", "h.hh", ".lll", ".llh", ".hll", ".hlh", // ┨┩┪┫┬┭┮┯
    ".lhl", ".lhh", ".hhl", ".hhh", "ll.l", "ll.h", "lh.l", "lh.h", // ┰┱┲┳┴┵┶┷
    "hl.l", "hl.h", "hh.l", "hh.h", "llll", "lllh", "lhll", "lhlh", // ┸┹┺┻┼┽┾┿
    "hlll", "llhl", "hlhl", "hllh", "hhll", "llhh", "lhhl", "hhlh", // ╀╁╂╃╄╅╆╇
    "lhhh", "hlhh", "hhhl", "hhhh", ".l.l", ".h.h", "l.l.", "h.h.", // ╈╉╊╋╌╍╎╏
    ".d.d", "d.d.", ".dl.", ".ld.", ".dd.", "..ld", "..dl", "..dd", // ═║╒╓╔╕╖╗
    "ld..", "dl..", "dd..", "l..d", "d..l", "d..d", "ldl.", "dld.", // ╘╙╚╛╜╝╞╟
    "ddd.", "l.ld", "d.dl", "d.dd", ".dld", ".ldl", ".ddd", "ld.d", // ╠╡╢╣╤╥╦╧
    "dl.l", "dd.d", "ldld", "dldl", "dddd", "", "", "",             // ╨╩╪╫╬╭╮╯
    "", "", "", "", "...l", "l...", ".l..", "..l.",                 // ╰╱╲╳╴╵╶╷
    "...h", "h...", ".h..", "..h.", ".h.l", "l.h.", ".l.h", "h.l.", // ╸╹╺╻╼╽╾╿
];

fn lines(character: char, cell: CellMetrics) -> Option<Vec<Shape>> {
    let arms = LINES[character as usize - 0x2500];
    if arms.is_empty() {
        return None;
    }
    let mut weights = arms.chars().map(|arm| match arm {
        'l' => Some(Weight::Light),
        'h' => Some(Weight::Heavy),
        'd' => Some(Weight::Double),
        _ => None,
    });
    let mut arm = || weights.next().flatten();
    let [up, right, down, left] = [arm(), arm(), arm(), arm()];
    let dashes = match character {
        '┄' | '┅' | '┆' | '┇' => 3,
        '┈' | '┉' | '┊' | '┋' => 4,
        '╌' | '╍' | '╎' | '╏' => 2,
        _ => 0,
    };
    if dashes > 0 {
        return Some(dashed(right.or(up)?, right.is_some(), dashes, cell));
    }

    let (width, height, light) = (cell.width, cell.height, cell.light);
    let mut shapes = Vec::new();
    for (weight, toward_end, opposite) in [(right, true, left), (left, false, right)] {
        let Some(weight) = weight else { continue };
        for (index, (top, bottom)) in spans(weight, height, light).into_iter().enumerate() {
            let end = inner_end(
                weight,
                index,
                [up, down],
                opposite,
                width,
                light,
                toward_end,
            );
            let (from, to) = if toward_end { (end, width) } else { (0., end) };
            shapes.push(filled(from, top, to - from, bottom - top));
        }
    }
    for (weight, toward_end, opposite) in [(down, true, up), (up, false, down)] {
        let Some(weight) = weight else { continue };
        for (index, (left_edge, right_edge)) in spans(weight, width, light).into_iter().enumerate()
        {
            let end = inner_end(
                weight,
                index,
                [left, right],
                opposite,
                height,
                light,
                toward_end,
            );
            let (from, to) = if toward_end { (end, height) } else { (0., end) };
            shapes.push(filled(left_edge, from, right_edge - left_edge, to - from));
        }
    }
    Some(shapes)
}

/// Where lines of `weight` lie across a cell `across` wide: one span, or two
/// for a double line, a light line's width apart.
fn spans(weight: Weight, across: f32, light: f32) -> Vec<(f32, f32)> {
    let span = |width: f32| {
        let start = ((across - width) / 2.).floor().max(0.);
        (start, start + width)
    };
    match weight {
        Weight::Light => vec![span(light)],
        Weight::Heavy => vec![span(light * 2.)],
        Weight::Double => {
            let (start, end) = span(light * 3.);
            vec![(start, start + light), (end - light, end)]
        }
    }
}

/// Where line `index` of an arm stops in the middle of a cell `along` long:
/// the arm runs from there to the far edge (`toward_end`, right or down) or
/// from the near edge to there. `sides` are the arms across it, before and
/// after it (up and down, or left and right); `opposite` the arm in line.
fn inner_end(
    weight: Weight,
    index: usize,
    sides: [Option<Weight>; 2],
    opposite: Option<Weight>,
    along: f32,
    light: f32,
    toward_end: bool,
) -> f32 {
    let across: Vec<Weight> = sides.into_iter().flatten().collect();
    if across.is_empty() {
        return (along / 2.).floor();
    }
    if across.contains(&Weight::Double) {
        // Lines meeting a double line stop at its near line, so as not to
        // cross between its two, or reach its far one to turn a corner.
        let double = spans(Weight::Double, along, light);
        let stops_near = match weight {
            Weight::Double => sides[index].is_some(),
            _ => sides.iter().all(|side| *side == Some(Weight::Double)) && opposite.is_none(),
        };
        return if toward_end {
            double[if stops_near { 1 } else { 0 }].0
        } else {
            double[if stops_near { 0 } else { 1 }].1
        };
    }
    // Across single lines, to their far side, for a square join.
    let crossing = across.into_iter().map(|side| spans(side, along, light)[0]);
    if toward_end {
        crossing.map(|span| span.0).fold(f32::MAX, f32::min)
    } else {
        crossing.map(|span| span.1).fold(f32::MIN, f32::max)
    }
}

/// A line of `count` dashes, each cell's starting half a gap in so they keep
/// their spacing from cell to cell.
fn dashed(weight: Weight, horizontal: bool, count: usize, cell: CellMetrics) -> Vec<Shape> {
    let (along, across) = if horizontal {
        (cell.width, cell.height)
    } else {
        (cell.height, cell.width)
    };
    let (start, end) = spans(weight, across, cell.light)[0];
    let step = along / count as f32;
    let gap = (step / 2.).round().max(1.);
    (0..count)
        .map(|dash| {
            let from = (step * dash as f32 + gap / 2.).round();
            let to = (step * (dash + 1) as f32 - gap / 2.).round().max(from + 1.);
            if horizontal {
                filled(from, start, to - from, end - start)
            } else {
                filled(start, from, end - start, to - from)
            }
        })
        .collect()
}

/// ╭╮╯╰: a light line from one edge bending round to another.
fn arc(character: char, cell: CellMetrics) -> Shape {
    let (left, right) = spans(Weight::Light, cell.width, cell.light)[0];
    let (top, bottom) = spans(Weight::Light, cell.height, cell.light)[0];
    let (width, height) = (cell.width - left, cell.height - top);
    let (rect, bend) = match character {
        '╭' => (rect(left, top, width * 2., height * 2.), Bend::TopLeft),
        '╮' => (rect(-right, top, right * 2., height * 2.), Bend::TopRight),
        '╯' => (
            rect(-right, -bottom, right * 2., bottom * 2.),
            Bend::BottomRight,
        ),
        _ => (
            rect(left, -bottom, width * 2., bottom * 2.),
            Bend::BottomLeft,
        ),
    };
    Shape::Arc {
        rect,
        bend,
        thickness: cell.light,
    }
}

/// Which quarters ▖▗▘▙▚▛▜▝▞▟ fill: upper left, upper right, lower left,
/// lower right.
const QUADRANTS: [[bool; 4]; 10] = [
    [false, false, true, false],
    [false, false, false, true],
    [true, false, false, false],
    [true, false, true, true],
    [true, false, false, true],
    [true, true, true, false],
    [true, true, false, true],
    [false, true, false, false],
    [false, true, true, false],
    [false, true, true, true],
];

fn blocks(character: char, cell: CellMetrics) -> Vec<Shape> {
    let (width, height) = (cell.width, cell.height);
    let eighths_high = |count: u32| (height * count as f32 / 8.).round();
    let eighths_wide = |count: u32| (width * count as f32 / 8.).round();
    let (middle_x, middle_y) = ((width / 2.).round(), (height / 2.).round());
    match character as u32 {
        0x2580 => vec![filled(0., 0., width, middle_y)],
        code @ 0x2581..=0x2588 => {
            let high = eighths_high(code - 0x2580);
            vec![filled(0., height - high, width, high)]
        }
        code @ 0x2589..=0x258F => vec![filled(0., 0., eighths_wide(0x2590 - code), height)],
        0x2590 => vec![filled(middle_x, 0., width - middle_x, height)],
        code @ 0x2591..=0x2593 => vec![Shape::Fill {
            rect: rect(0., 0., width, height),
            alpha: (code - 0x2590) as f32 / 4.,
        }],
        0x2594 => vec![filled(0., 0., width, eighths_high(1))],
        0x2595 => {
            let wide = eighths_wide(1);
            vec![filled(width - wide, 0., wide, height)]
        }
        code => {
            let quarters = [
                (0., 0., middle_x, middle_y),
                (middle_x, 0., width - middle_x, middle_y),
                (0., middle_y, middle_x, height - middle_y),
                (middle_x, middle_y, width - middle_x, height - middle_y),
            ];
            QUADRANTS[(code - 0x2596) as usize]
                .iter()
                .zip(quarters)
                .filter(|(filled, _)| **filled)
                .map(|(_, (x, y, w, h))| filled(x, y, w, h))
                .collect()
        }
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<f32> {
    Bounds::new(point(x, y), size(width, height))
}

fn filled(x: f32, y: f32, width: f32, height: f32) -> Shape {
    Shape::Fill {
        rect: rect(x, y, width, height),
        alpha: 1.,
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::{CellMetrics, Shape, shapes};

    /// A cell 7 × 9 pixels with lines of one pixel.
    const CELL: CellMetrics = CellMetrics {
        width: 7.,
        height: 9.,
        light: 1.,
    };

    /// `character` drawn in [`CELL`], `#` for each pixel filled.
    fn drawn(character: char) -> Vec<String> {
        let mut pixels = vec![vec!['.'; 7]; 9];
        for shape in shapes(character, CELL).unwrap() {
            let Shape::Fill { rect, .. } = shape else {
                panic!("not filled: {shape:?}");
            };
            let columns = rect.left() as usize..rect.right() as usize;
            for row in &mut pixels[rect.top() as usize..rect.bottom() as usize] {
                row[columns.clone()].fill('#');
            }
        }
        pixels.into_iter().map(String::from_iter).collect()
    }

    #[test]
    fn a_vertical_line_runs_the_height_of_the_cell_to_meet_the_next() {
        assert_eq!(drawn('│'), ["...#..."; 9]);
        assert_eq!(drawn('─')[4], "#######");
    }

    #[test]
    fn lines_join_square_and_double_lines_turn_both_corners() {
        assert_eq!(
            drawn('┌'),
            [
                ".......", ".......", ".......", ".......", "...####", "...#...", "...#...",
                "...#...", "...#...",
            ]
        );
        assert_eq!(
            drawn('╔'),
            [
                ".......", ".......", ".......", "..#####", "..#....", "..#.###", "..#.#..",
                "..#.#..", "..#.#..",
            ]
        );
        assert_eq!(
            drawn('╬'),
            [
                "..#.#..", "..#.#..", "..#.#..", "###.###", ".......", "###.###", "..#.#..",
                "..#.#..", "..#.#..",
            ]
        );
        // A single line crossing a double one goes through it, one meeting
        // it from one side stops at it.
        assert_eq!(drawn('╫')[4], "#######");
        assert_eq!(drawn('╟')[4], "..#.###");
    }

    #[test]
    fn half_blocks_fill_the_cell_between_them() {
        let upper = drawn('▀');
        let lower = drawn('▄');
        for (upper, lower) in upper.iter().zip(&lower) {
            assert!(upper == "#######" || lower == "#######");
        }
        assert_eq!(drawn('█'), ["#######"; 9]);
        assert_eq!(drawn('▚')[0], "####...");
        assert_eq!(drawn('▚')[8], "....###");
    }

    #[test]
    fn diagonals_and_other_characters_are_left_to_the_font() {
        for character in ['╱', '╳', 'a', '中', '■'] {
            assert_eq!(shapes(character, CELL), None);
        }
        assert!(shapes('╭', CELL).is_some());
        assert!(shapes('┄', CELL).is_some());
    }

    #[test]
    fn lines_are_a_pixel_at_13_px_and_two_at_150_percent() {
        let light = |scale| CellMetrics::new(px(8.), px(20.), px(13.), scale).light;
        assert_eq!(light(1.), 1.);
        assert_eq!(light(1.5), 2.);
        assert_eq!(light(2.), 2.);
    }
}
