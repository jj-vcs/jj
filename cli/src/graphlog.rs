// Copyright 2020 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fmt::Debug;
use std::hash::Hash;
use std::io;

use jj_lib::config::ConfigGetError;
use jj_lib::graph::GraphEdge;
use jj_lib::graph::GraphEdgeType;
use jj_lib::settings::UserSettings;
use renderdag::Ancestor;
use renderdag::pipeline::Ascii;
use renderdag::pipeline::AsciiLarge;
use renderdag::pipeline::BoxDrawing;
use renderdag::pipeline::GraphRowShaper;
use renderdag::pipeline::PrefixLineRenderer;
use renderdag::pipeline::PrefixLinesToText;
use renderdag::pipeline::coloring::build_line_id_allocator_from_rules;
use renderdag::pipeline::row_shape_to_prefix_lines::box_drawing::Curved;
use renderdag::pipeline::row_shape_to_prefix_lines::box_drawing::Square;

use crate::formatter::Formatter;
use crate::formatter::FormatterExt as _;

/// Color labels for graph lines, under the `graph` label. A line has the label
/// that matches its line ID. Like Git, we support 12 styles before wrapping around.
const LANE_LABELS: [&str; 12] = [
    "lane0", "lane1", "lane2", "lane3", "lane4", "lane5", "lane6", "lane7", "lane8", "lane9",
    "lane10", "lane11",
];

pub trait GraphLog<K: Clone + Eq + Hash> {
    fn add_node(
        &mut self,
        id: &K,
        edges: &[GraphEdge<K>],
        node_symbol: &str,
        text: &str,
    ) -> io::Result<()>;

    fn width(&self, id: &K, edges: &[GraphEdge<K>]) -> usize;
}

pub struct SaplingGraphLog<'writer, K> {
    shaper: GraphRowShaper<K>,
    prefix_lines: Box<dyn PrefixLineRenderer<K>>,
    text: PrefixLinesToText,
    /// Converts a width in columns to a width in characters.
    columns_to_chars: fn(u64) -> u64,
    formatter: &'writer mut dyn Formatter,
}

fn convert_graph_edge_into_ancestor<K: Clone>(e: &GraphEdge<K>) -> Ancestor<K> {
    match e.edge_type {
        GraphEdgeType::Direct => Ancestor::Parent(e.target.clone()),
        GraphEdgeType::Indirect => Ancestor::Ancestor(e.target.clone()),
        GraphEdgeType::Missing => Ancestor::Anonymous,
    }
}

impl<K> GraphLog<K> for SaplingGraphLog<'_, K>
where
    K: Clone + Debug + Eq + Hash,
{
    fn add_node(
        &mut self,
        id: &K,
        edges: &[GraphEdge<K>],
        node_symbol: &str,
        text: &str,
    ) -> io::Result<()> {
        let row_shape = self.shaper.next_row_shape(
            id.clone(),
            edges.iter().map(convert_graph_edge_into_ancestor).collect(),
        );
        let prefix_lines = self.prefix_lines.next_prefix_lines(&row_shape);
        let mut result = Ok(());
        self.text.write_next_lines(
            |text, color| {
                if result.is_ok() {
                    result = write_piece(self.formatter, text, *color);
                }
            },
            prefix_lines,
            row_shape.separator_line,
            node_symbol,
            text,
            self.shaper.options().min_row_height,
        );
        result
    }

    fn width(&self, id: &K, edges: &[GraphEdge<K>]) -> usize {
        let parents = edges.iter().map(convert_graph_edge_into_ancestor).collect();
        let columns = self.shaper.width(Some(id), Some(&parents));
        (self.columns_to_chars)(columns).try_into().unwrap()
    }
}

/// Writes one piece of graph text, with the color label of its line.
fn write_piece(
    formatter: &mut dyn Formatter,
    text: &str,
    line_id: Option<usize>,
) -> io::Result<()> {
    match line_id {
        Some(id) => {
            // Leave the space after a glyph unlabeled, so that a lane
            // background color covers only the glyph.
            let glyph = text.trim_end();
            let space = &text[glyph.len()..];
            let lane = LANE_LABELS[id % LANE_LABELS.len()];
            write!(formatter.labeled("graph").labeled(lane), "{glyph}")?;
            formatter.raw()?.write_all(space.as_bytes())
        }
        // The node symbol, the message text, and the line breaks are already
        // formatted.
        None => formatter.raw()?.write_all(text.as_bytes()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub enum GraphStyle {
    Ascii,
    AsciiLarge,
    Curved,
    Square,
}

impl GraphStyle {
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        settings.get("ui.graph.style")
    }
}

pub fn get_graphlog<'a, K: Clone + Debug + Eq + Hash + 'a>(
    style: GraphStyle,
    formatter: &'a mut dyn Formatter,
) -> Box<dyn GraphLog<K> + 'a> {
    let prefix_lines: Box<dyn PrefixLineRenderer<K>> = match style {
        GraphStyle::Ascii => Box::new(Ascii::new()),
        GraphStyle::AsciiLarge => Box::new(AsciiLarge),
        GraphStyle::Curved => Box::new(BoxDrawing::<Curved>::new()),
        GraphStyle::Square => Box::new(BoxDrawing::<Square>::new()),
    };
    let columns_to_chars: fn(u64) -> u64 = match style {
        // The first column is only 2 characters wide.
        GraphStyle::AsciiLarge => |columns| {
            columns
                .saturating_mul(3)
                .saturating_sub(1)
                .saturating_add(1)
        },
        _ => |columns| columns.saturating_mul(2).saturating_add(1),
    };
    let allocator = build_line_id_allocator_from_rules("parent,incoming,free", LANE_LABELS.len());
    let mut shaper = GraphRowShaper::new().with_line_id_allocator(allocator);
    shaper.options_mut().min_row_height = 0;
    Box::new(SaplingGraphLog {
        shaper,
        prefix_lines,
        text: PrefixLinesToText::new(),
        columns_to_chars,
        formatter,
    })
}
