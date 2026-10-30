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
use std::io::Write as _;

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
use renderdag::pipeline::row_shape_to_prefix_lines::box_drawing::Curved;
use renderdag::pipeline::row_shape_to_prefix_lines::box_drawing::Square;
use renderdag::pipeline::types::LinePiece;

use crate::formatter::Formatter;
use crate::formatter::FormatterExt as _;
use crate::ui::Ui;

/// Color labels for graph columns, under the `graph` label. Columns reuse the
/// labels in turn.
const LANE_LABELS: [&str; 12] = [
    "lane0", "lane1", "lane2", "lane3", "lane4", "lane5", "lane6", "lane7", "lane8", "lane9",
    "lane10", "lane11",
];

pub trait GraphLog<K: Clone + Eq + Hash> {
    fn add_node(
        &mut self,
        id: &K,
        edges: &[GraphEdge<K>],
        render_node_symbol: &dyn Fn(&mut dyn Formatter) -> io::Result<()>,
        text: &str,
    ) -> io::Result<()>;

    fn width(&self, id: &K, edges: &[GraphEdge<K>]) -> usize;
}

pub struct SaplingGraphLog<'a, K> {
    shaper: GraphRowShaper<K>,
    prefix_lines: Box<dyn PrefixLineRenderer<K>>,
    text: PrefixLinesToText,
    /// Converts a width in columns to a width in characters.
    columns_to_chars: fn(u64) -> u64,
    formatter: &'a mut dyn Formatter,
    ui: &'a Ui,
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
        render_node_symbol: &dyn Fn(&mut dyn Formatter) -> io::Result<()>,
        text: &str,
    ) -> io::Result<()> {
        let row_shape = self.shaper.next_row_shape(
            id.clone(),
            edges.iter().map(convert_graph_edge_into_ancestor).collect(),
        );
        let prefix_lines = self.prefix_lines.next_prefix_lines(&row_shape);
        // Render the node symbol in the color of its column, unless the node
        // labels set their own color.
        let node_column = row_shape.node_column;
        let mut glyph = vec![];
        let mut formatter = self.ui.new_formatter(&mut glyph);
        formatter.push_label("graph");
        formatter.push_label(lane_label(node_column));
        render_node_symbol(formatter.as_mut())?;
        formatter.pop_label();
        formatter.pop_label();
        drop(formatter);
        let mut result = Ok(());
        self.text.write_next_lines(
            |line| {
                if result.is_ok() {
                    result = write_line(self.formatter, line);
                }
            },
            prefix_lines,
            row_shape.separator_line,
            &String::from_utf8_lossy(&glyph),
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

/// The color label for a graph column.
fn lane_label(column: usize) -> &'static str {
    LANE_LABELS[column % LANE_LABELS.len()]
}

/// Writes one graph line, with a color label for each column.
fn write_line(formatter: &mut dyn Formatter, line: &[LinePiece<'_>]) -> io::Result<()> {
    for piece in line {
        match piece.column {
            Some(column) => {
                let lane = lane_label(column);
                write!(formatter.labeled("graph").labeled(lane), "{}", piece.text)?;
            }
            // The node symbol and the message text are already formatted.
            None => formatter.raw()?.write_all(piece.text.as_bytes())?,
        }
    }
    writeln!(formatter.raw()?)
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
    ui: &'a Ui,
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
    let mut shaper = GraphRowShaper::new();
    shaper.options_mut().min_row_height = 0;
    Box::new(SaplingGraphLog {
        shaper,
        prefix_lines,
        text: PrefixLinesToText::new(),
        columns_to_chars,
        formatter,
        ui,
    })
}
