//! Plot styles: the pens a plot draws with, one for each AutoCAD colour
//! index, each giving the colour and lineweight the index plots in.
//!
//! A plot style comes from a CTB file, AutoCAD's colour-dependent plot
//! style table, such as AutoCAD's own `monochrome.ctb`. AutoCAD's CTB files
//! are Autodesk's and never enter this repository (ADR-0008): they are read
//! from the plot style folders a setting names, as fonts are, and tests
//! write CTB files of their own. With no CTB named, a plot draws with the
//! built-in monochrome style: every pen black, at the object's own
//! lineweight, as `monochrome.ctb` plots.
//!
//! A CTB file is a 48-byte heading, `PIAFILEVERSION_2.0,CTBVER1,compress`
//! and `pmzlibcodec` on two lines, then three little-endian 32-bit numbers
//! (the Adler-32 of what follows, its length unpacked and its length
//! packed), then zlib-packed text, which some writers, such as ezdxf, end
//! with a NUL byte. The text holds nested sections of
//! `key=value` lines: `plot_style` holds a section per colour index, `0` for
//! index 1 up to `254` for index 255, and `custom_lineweight_table` the
//! lineweights in millimetres that a pen's `lineweight` counts into.
//!
//! What a pen sets:
//!
//! - `color`: -1, or -1006632961, plots the object's own colour; any other
//!   value means the pen plots in a colour of its own, red, green and blue
//!   in the low three bytes of its `mode_color`. CTB writers keep a custom
//!   colour there, and `color` may then hold another value; a pen with no
//!   `mode_color` plots the colour in `color`'s low three bytes. A
//!   `mode_color` of -1006632961, or -1, plots the object's own colour. A
//!   value whose top byte is 0xC3, which ezdxf calls ACI, is red, green
//!   and blue too, not a colour index. AutoCAD 2026 plotted a made-up CTB
//!   so, pen by pen.
//! - `lineweight`: 0 plots the object's own lineweight; any other value `n`
//!   plots with the lineweight at place `n - 1` of `custom_lineweight_table`,
//!   counted from 0, so 1 is the table's first, 0 mm. AutoCAD 2026 plotted a
//!   made-up CTB's pens of `lineweight` 20 and 25 at the table's 0.9 and
//!   1.58 mm, its places 19 and 24.
//!
//! A pen's other settings, such as screening, grayscale, a linetype or a
//! line end style, are not followed yet: the plot names each pen that sets
//! one, so nothing is left out in silence.

use serde::Serialize;

use crate::draw::Color;

/// The heading every CTB file starts with, before its three numbers.
const CTB_HEADING: &[u8] = b"PIAFILEVERSION_2.0,CTBVER1,compress\r\npmzlibcodec";

/// The heading of a named plot style table, which this does not read.
const STB_HEADING: &[u8] = b"PIAFILEVERSION_2.0,STBVER1,compress\r\npmzlibcodec";

/// How many bytes come before the packed text.
const BEFORE_TEXT: usize = 60;

/// The most bytes the packed text may unpack to. AutoCAD's own CTB files
/// unpack to under 100 kB.
const MOST_TEXT: usize = 16 * 1024 * 1024;

/// The colours that mean a pen plots the object's own colour.
const OBJECT_COLORS: [i64; 2] = [-1, -1_006_632_961];

/// What a plot style sets for one colour index.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Pen {
    /// The colour the index plots in; none for the object's own colour.
    pub color: Option<[u8; 3]>,
    /// The lineweight in millimetres the index plots with; none for the
    /// object's own lineweight.
    pub lineweight: Option<f64>,
}

/// The pens a plot draws with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlotStyle {
    /// Pens by colour index; index 0 is never drawn.
    pens: Vec<Pen>,
    /// The pen settings this plot style sets that are not followed yet.
    pub not_followed: Vec<NotFollowed>,
}

/// A pen setting a plot style sets that AgentDrafter does not follow yet,
/// and the colour indexes whose pens set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NotFollowed {
    /// The setting, as the CTB file names it, such as `screen`.
    pub setting: String,
    /// What it is, in words, and what the plot does instead.
    pub meaning: String,
    /// The colour indexes whose pens set it.
    pub colors: Vec<u8>,
}

impl PlotStyle {
    /// The built-in monochrome plot style: every pen black, at the object's
    /// own lineweight, as AutoCAD's `monochrome.ctb` plots.
    pub(crate) fn monochrome() -> PlotStyle {
        PlotStyle {
            pens: vec![
                Pen {
                    color: Some([0, 0, 0]),
                    lineweight: None,
                };
                256
            ],
            not_followed: Vec::new(),
        }
    }

    /// The colour an object of colour index `color` plots in.
    pub(crate) fn color(&self, color: Color) -> [u8; 3] {
        self.pen(color).color.unwrap_or_else(|| color.on_paper())
    }

    /// The lineweight in millimetres an object of colour index `color` and
    /// lineweight `own` plots with.
    pub(crate) fn lineweight(&self, color: Color, own: f64) -> f64 {
        self.pen(color).lineweight.unwrap_or(own)
    }

    fn pen(&self, color: Color) -> Pen {
        self.pens[usize::from(color.0)]
    }

    /// Reads a CTB file's bytes. The error says why they cannot be read.
    pub(crate) fn parse_ctb(bytes: &[u8]) -> Result<PlotStyle, String> {
        if bytes.starts_with(STB_HEADING) {
            return Err(
                "it is a named plot style table (STB); a plot reads a colour-dependent \
                        one (CTB)"
                    .to_owned(),
            );
        }
        if !bytes.starts_with(CTB_HEADING) || bytes.len() < BEFORE_TEXT {
            return Err("it is not a CTB plot style table: it does not start as one".to_owned());
        }
        let packed = &bytes[BEFORE_TEXT..];
        let text = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(packed, MOST_TEXT)
            .map_err(|e| format!("its packed text cannot be unpacked ({:?})", e.status))?;
        // Some writers end the text with a NUL byte, which ends nothing.
        let end = text
            .iter()
            .rposition(|&b| b != 0)
            .map_or(0, |last| last + 1);
        // The text is single-byte; names may hold bytes above 127.
        let text: String = text[..end].iter().map(|&b| char::from(b)).collect();
        PlotStyle::from_text(&text)
    }

    /// Reads a CTB file's unpacked text.
    fn from_text(text: &str) -> Result<PlotStyle, String> {
        let root = Section::parse(text)?;
        let styles = root
            .section("plot_style")
            .ok_or("it has no `plot_style` section of pens")?;
        let lineweights: Vec<(usize, f64)> = match root.section("custom_lineweight_table") {
            Some(table) => table
                .values()
                .map(|(key, value)| {
                    let at = key.parse::<usize>().ok();
                    let mm = value
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite() && *v >= 0.0);
                    at.zip(mm)
                        .ok_or_else(|| format!("its lineweight table has `{key}={value}`"))
                })
                .collect::<Result<_, _>>()?,
            None => Vec::new(),
        };
        let apply_factor = root
            .value("apply_factor")
            .is_some_and(|v| v.eq_ignore_ascii_case("TRUE"));

        let mut style = PlotStyle {
            pens: vec![Pen::default(); 256],
            not_followed: Vec::new(),
        };
        let mut unfollowed: Vec<(&'static str, &'static str, Vec<u8>)> = Vec::new();
        let mut note = |setting: &'static str, meaning: &'static str, color: u8| match unfollowed
            .iter_mut()
            .find(|(s, _, _)| *s == setting)
        {
            Some((_, _, colors)) => colors.push(color),
            None => unfollowed.push((setting, meaning, vec![color])),
        };
        for (key, pen) in styles.sections() {
            let place: usize = key
                .parse()
                .ok()
                .filter(|p| *p < 255)
                .ok_or_else(|| format!("its pens run from 0 to 254, and one is `{key}`"))?;
            let index = place + 1;
            let color = u8::try_from(index).expect("a colour index is at most 255");
            let number = |name: &str| -> Result<Option<i64>, String> {
                pen.value(name)
                    .map(|v| {
                        v.trim().parse::<i64>().map_err(|_| {
                            format!("pen {key} has `{name}={v}`, which is not a whole number")
                        })
                    })
                    .transpose()
            };
            let plotted = match number("color")? {
                None => None,
                Some(c) if OBJECT_COLORS.contains(&c) => None,
                // A custom colour is kept in `mode_color`, and `color` may
                // then hold another value: AutoCAD plots `mode_color`. A
                // `mode_color` that names the object's own colour plots
                // that, as AutoCAD 2026 plotted one of -1006632961.
                Some(c) => match number("mode_color")?.unwrap_or(c) {
                    c if OBJECT_COLORS.contains(&c) => None,
                    c => {
                        let c = c as u32;
                        Some([(c >> 16) as u8, (c >> 8) as u8, c as u8])
                    }
                },
            };
            let lineweight = match number("lineweight")? {
                None | Some(0) => None,
                Some(at) => {
                    let mm = lineweights
                        .iter()
                        .find(|(i, _)| i64::try_from(*i + 1) == Ok(at))
                        .map(|&(_, mm)| mm)
                        .ok_or_else(|| {
                            format!(
                                "pen {key} has lineweight {at}, which its lineweight table \
                                 does not hold"
                            )
                        })?;
                    Some(mm)
                }
            };
            style.pens[index] = Pen {
                color: plotted,
                lineweight,
            };
            let policy = number("color_policy")?.unwrap_or(1);
            if policy & 2 != 0 {
                note(
                    "color_policy",
                    "grayscale: the pen plots its colour as a grey; here it plots in its colour",
                    color,
                );
            }
            let checks: [(&'static str, i64, &'static str); 5] = [
                (
                    "screen",
                    100,
                    "screening: the pen plots its colour lightened; here it plots at full strength",
                ),
                (
                    "linetype",
                    31,
                    "a linetype of the plot style's own; here the object's own linetype draws",
                ),
                ("end_style", 4, "a line end style; here line ends are round"),
                ("join_style", 5, "a line join style; here corners are round"),
                ("fill_style", 73, "a fill style; here areas fill solid"),
            ];
            for (setting, object, meaning) in checks {
                if number(setting)?.is_some_and(|v| v != object) {
                    note(setting, meaning, color);
                }
            }
        }
        if apply_factor {
            note(
                "apply_factor",
                "the plot style's scale factor for linetypes and fill patterns; here they keep \
                 their own sizes",
                0,
            );
        }
        style.not_followed = unfollowed
            .into_iter()
            .map(|(setting, meaning, mut colors)| {
                colors.retain(|&c| c != 0);
                NotFollowed {
                    setting: setting.to_owned(),
                    meaning: meaning.to_owned(),
                    colors,
                }
            })
            .collect();
        Ok(style)
    }
}

/// A section of a CTB file's text: its `key=value` lines and the sections
/// inside it, in order.
#[derive(Debug, Default)]
struct Section {
    entries: Vec<(String, Entry)>,
}

#[derive(Debug)]
enum Entry {
    Value(String),
    Section(Section),
}

impl Section {
    /// Reads the text of a CTB file into its sections.
    fn parse(text: &str) -> Result<Section, String> {
        let mut stack: Vec<(String, Section)> = vec![(String::new(), Section::default())];
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == "}" {
                if stack.len() < 2 {
                    return Err(format!(
                        "its line {} closes a section never opened",
                        number + 1
                    ));
                }
                let (name, section) = stack.pop().expect("an open section");
                let parent = &mut stack.last_mut().expect("the root").1;
                parent.entries.push((name, Entry::Section(section)));
            } else if let Some(name) = line.strip_suffix('{') {
                stack.push((name.trim().to_owned(), Section::default()));
            } else if let Some((key, value)) = line.split_once('=') {
                let value = match value.strip_prefix('"') {
                    // A text value opens with a quote that is never closed.
                    Some(text) => text,
                    // Any other value may carry an appendix after a space,
                    // in brackets, such as `0.0076200000000 (+7.Z+"8V?S_LC )`
                    // in a lineweight table (ezdxf's issue #1069). What it
                    // means is not known; the value is what comes before
                    // it, as ezdxf reads it and AutoCAD 2026 plotted it.
                    // Anything else after a space stays in the value,
                    // which then does not read.
                    None => match value.split_once(' ') {
                        Some((before, appendix))
                            if appendix.trim_start().starts_with('(')
                                && appendix.ends_with(')') =>
                        {
                            before
                        }
                        _ => value,
                    },
                };
                let section = &mut stack.last_mut().expect("the root").1;
                section
                    .entries
                    .push((key.trim().to_owned(), Entry::Value(value.to_owned())));
            } else {
                return Err(format!(
                    "its line {} is neither `key=value`, a section's start nor its end",
                    number + 1
                ));
            }
        }
        if stack.len() != 1 {
            return Err("a section in it is never closed".to_owned());
        }
        Ok(stack.pop().expect("the root").1)
    }

    fn value(&self, key: &str) -> Option<&str> {
        self.entries.iter().find_map(|(k, e)| match e {
            Entry::Value(v) if k == key => Some(v.as_str()),
            _ => None,
        })
    }

    fn section(&self, key: &str) -> Option<&Section> {
        self.entries.iter().find_map(|(k, e)| match e {
            Entry::Section(s) if k == key => Some(s),
            _ => None,
        })
    }

    fn values(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().filter_map(|(k, e)| match e {
            Entry::Value(v) => Some((k.as_str(), v.as_str())),
            Entry::Section(_) => None,
        })
    }

    fn sections(&self) -> impl Iterator<Item = (&str, &Section)> {
        self.entries.iter().filter_map(|(k, e)| match e {
            Entry::Section(s) => Some((k.as_str(), s)),
            Entry::Value(_) => None,
        })
    }
}

/// The made-up CTB files the tests write, from the one writer the
/// integration tests use too.
#[cfg(test)]
#[path = "../tests/support/ctb.rs"]
mod ctb;

#[cfg(test)]
mod tests {
    use super::ctb::{ctb_file, made_up_ctb_text, rgb, BLACK, OBJECT_COLOR};
    use super::*;

    #[test]
    fn a_ctb_sets_the_colour_and_lineweight_of_each_pen() {
        // Pen 1 plots blue at 1 mm, the table's place 20; pen 2 its own
        // colour at 0.5 mm, place 13; pen 3 its own colour and lineweight;
        // pen 4 at 0 mm, the table's first; the rest black at their own.
        let text = made_up_ctb_text(
            &[
                (1, rgb(0, 0, 255), 21),
                (2, OBJECT_COLOR, 14),
                (3, OBJECT_COLOR, 0),
                (4, BLACK, 1),
            ],
            "",
        );
        let style = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap();
        assert_eq!(style.color(Color(1)), [0, 0, 255]);
        assert_eq!(style.lineweight(Color(1), 0.25), 1.0);
        assert_eq!(style.color(Color(2)), Color(2).on_paper());
        assert_eq!(style.lineweight(Color(2), 0.25), 0.5);
        assert_eq!(style.color(Color(3)), Color(3).on_paper());
        assert_eq!(style.lineweight(Color(3), 0.7), 0.7);
        assert_eq!(style.color(Color(4)), [0, 0, 0]);
        assert_eq!(style.lineweight(Color(4), 0.35), 0.0);
        assert_eq!(style.color(Color(7)), [0, 0, 0]);
        assert_eq!(style.lineweight(Color(255), 0.35), 0.35);
        assert_eq!(style.not_followed, Vec::new());
    }

    #[test]
    fn the_built_in_monochrome_style_is_every_pen_black_at_its_own_lineweight() {
        let style = PlotStyle::monochrome();
        let ctb = PlotStyle::parse_ctb(&ctb_file(&made_up_ctb_text(&[], ""))).unwrap();
        for index in 1..=255u8 {
            assert_eq!(style.color(Color(index)), [0, 0, 0]);
            assert_eq!(style.lineweight(Color(index), 0.13), 0.13);
            assert_eq!(ctb.pen(Color(index)), style.pen(Color(index)));
        }
    }

    #[test]
    fn the_object_colour_is_written_two_ways() {
        let text = made_up_ctb_text(&[(5, -1_006_632_961, 0)], "");
        let style = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap();
        assert_eq!(style.color(Color(5)), [0, 0, 255]);
    }

    #[test]
    fn a_ctb_ending_in_a_nul_byte_is_read() {
        // ezdxf's writer ends the packed text with a NUL byte.
        let text = made_up_ctb_text(&[(1, rgb(0, 0, 255), 21)], "") + "\0";
        let style = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap();
        assert_eq!(style.color(Color(1)), [0, 0, 255]);
        assert_eq!(style.lineweight(Color(1), 0.25), 1.0);
        assert_eq!(style.color(Color(2)), [0, 0, 0]);
    }

    #[test]
    fn a_custom_colour_is_read_from_mode_color() {
        // Pen 1's `color` holds red and its `mode_color` blue, which it
        // plots in; pen 2 has no `mode_color` and plots in its `color`;
        // pen 3 plots the object's own colour whatever its `mode_color`.
        let red = rgb(255, 0, 0);
        let text = made_up_ctb_text(
            &[(1, red, 0), (2, rgb(0, 128, 0), 0), (3, OBJECT_COLOR, 0)],
            "",
        )
        .replacen(
            &format!("  color={red}\n"),
            &format!("  color={red}\n  mode_color={}\n", rgb(0, 0, 255)),
            1,
        )
        .replacen(
            &format!("  color={OBJECT_COLOR}\n"),
            &format!("  color={OBJECT_COLOR}\n  mode_color={}\n", rgb(9, 9, 9)),
            1,
        );
        assert_eq!(text.matches("mode_color").count(), 2);
        let style = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap();
        assert_eq!(style.color(Color(1)), [0, 0, 255]);
        assert_eq!(style.color(Color(2)), [0, 128, 0]);
        assert_eq!(style.color(Color(3)), Color(3).on_paper());
    }

    #[test]
    fn pens_plot_as_autocad_plotted_the_probe() {
        // The pens of the made-up CTB AutoCAD 2026 plotted for #36, each
        // `(index, color, mode_color)`, and the colour AutoCAD plotted it in.
        let aci = |index: u32| i64::from((0xC300_0000u32 | index) as i32);
        let object = aci(0x00FF_FFFF);
        assert_eq!(object, -1_006_632_961);
        let (red, blue, green) = (rgb(255, 0, 0), rgb(0, 0, 255), rgb(0, 128, 0));
        let pens: [(u8, i64, Option<i64>, [u8; 3]); 10] = [
            // `color` and `mode_color` differ: `mode_color`.
            (1, red, Some(blue), [0, 0, 255]),
            (30, aci(3), Some(blue), [0, 0, 255]),
            // A value of the 0xC3 kind is red, green and blue, not an index.
            (2, red, Some(aci(3)), [0, 0, 3]),
            (3, aci(5), None, [0, 0, 5]),
            (50, aci(1), Some(aci(1)), [0, 0, 1]),
            (60, aci(2), Some(aci(4)), [0, 0, 4]),
            // A `mode_color` naming the object's own colour: index 4's cyan.
            (4, red, Some(object), [0, 255, 255]),
            // A `color` naming it, whatever the `mode_color`: index 6's
            // magenta.
            (6, OBJECT_COLOR, Some(green), [255, 0, 255]),
            (40, green, Some(green), [0, 128, 0]),
            (250, red, None, [255, 0, 0]),
        ];
        let mut text = made_up_ctb_text(&pens.map(|(index, color, _, _)| (index, color, 0)), "");
        for (index, _, mode, _) in pens {
            if let Some(mode) = mode {
                let name = format!("  name=\"Color_{index}\n");
                let at = text.find(&name).unwrap() + name.len();
                text.insert_str(at, &format!("  mode_color={mode}\n"));
            }
        }
        let style = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap();
        for (index, _, _, plotted) in pens {
            assert_eq!(style.color(Color(index)), plotted, "pen {index}");
        }
    }

    #[test]
    fn a_value_with_an_appendix_is_read_as_the_value_before_it() {
        // As ezdxf's issue #1069 shows them in a lineweight table: the
        // table's place 19, 0.9 mm, and place 20, 1 mm, carry appendices,
        // and pens 1 and 2 plot with them.
        let text = made_up_ctb_text(&[(1, rgb(0, 0, 255), 20), (2, BLACK, 21)], "")
            .replacen(" 19=0.9\n", " 19=0.9000000000 (+7.Z+\"8V?S_LC )\n", 1)
            .replacen(" 20=1.0\n", " 20=1.0000000000 (44F=@\";\"QC\\SM0)\n", 1);
        assert_eq!(text.matches(" (").count(), 2, "both appendices written");
        let style = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap();
        assert_eq!(style.lineweight(Color(1), 0.25), 0.9);
        assert_eq!(style.lineweight(Color(2), 0.25), 1.0);
        assert_eq!(style.color(Color(1)), [0, 0, 255]);
        // Anything else after the value is still refused: brackets that do
        // not follow a space, and words that are not in brackets.
        for written in ["0.9(x)", "0.9 garbage)", "0.9 (x) garbage", "0.9 x (y)"] {
            let text =
                made_up_ctb_text(&[], "").replacen(" 19=0.9\n", &format!(" 19={written}\n"), 1);
            let error = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap_err();
            assert!(
                error.contains(&format!("19={written}")),
                "{written}: {error}"
            );
        }
    }

    #[test]
    fn settings_not_followed_are_named_with_their_pens() {
        let text = made_up_ctb_text(&[], "  screen=50\n").replacen(
            "  screen=100\n  linepattern_size=0.5\n",
            "  linepattern_size=0.5\n",
            255,
        );
        let style = PlotStyle::parse_ctb(&ctb_file(&text)).unwrap();
        assert_eq!(style.not_followed.len(), 1, "{:?}", style.not_followed);
        assert_eq!(style.not_followed[0].setting, "screen");
        assert_eq!(style.not_followed[0].colors.len(), 255);
    }

    #[test]
    fn what_is_not_a_ctb_is_refused_with_why() {
        assert!(PlotStyle::parse_ctb(b"not a plot style")
            .unwrap_err()
            .contains("not a CTB"));
        let mut stb = ctb_file(&made_up_ctb_text(&[], ""));
        stb[19..26].copy_from_slice(b"STBVER1");
        assert!(PlotStyle::parse_ctb(&stb).unwrap_err().contains("STB"));
        let mut broken = ctb_file(&made_up_ctb_text(&[], ""));
        broken.truncate(200);
        assert!(PlotStyle::parse_ctb(&broken)
            .unwrap_err()
            .contains("cannot be unpacked"));
        let unknown = made_up_ctb_text(&[(1, OBJECT_COLOR, 28)], "");
        assert!(PlotStyle::parse_ctb(&ctb_file(&unknown))
            .unwrap_err()
            .contains("lineweight 28"));
    }
}
