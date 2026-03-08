// src/vision/layout.rs
//
// Использует `crate::config::LayoutConfig` вместо локального определения.
// Пороги ax_tree_min_useful_nodes и ax_tree_min_coverage_ratio приходят как параметры.

use serde::Deserialize;
use std::collections::VecDeque;
use crate::config::LayoutConfig;

// ─── Входные данные от ax-helper ──────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct FrameData {
    pub x: f64, pub y: f64, pub w: f64, pub h: f64,
}

#[derive(Debug, Deserialize)]
pub struct AXNode {
    pub role:        String,
    pub title:       Option<String>,
    pub value:       Option<String>,
    pub description: Option<String>,
    pub frame:       Option<FrameData>,
    pub children:    Option<Vec<AXNode>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OCRNode {
    pub text:  String,
    pub frame: FrameData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpOutput {
    pub ax_tree:  Option<AXNode>,
    pub ocr_text: Vec<OCRNode>,
}

// ─── Аннотатор ────────────────────────────────────────────────────────────────

pub trait BlockAnnotator {
    fn annotate_block(&self, block: &LayoutBlock) -> String;
    fn annotate_line(&self, line: &TextLine) -> String;
}

pub struct NoopAnnotator;

impl BlockAnnotator for NoopAnnotator {
    fn annotate_block(&self, _: &LayoutBlock) -> String { String::new() }
    fn annotate_line(&self, _: &TextLine)     -> String { String::new() }
}

// ─── Внутренние типы геометрии ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BoundingBox { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }

impl BoundingBox {
    pub fn right(&self)    -> f64 { self.x + self.w }
    pub fn bottom(&self)   -> f64 { self.y + self.h }
    pub fn center_x(&self) -> f64 { self.x + self.w / 2.0 }
    pub fn center_y(&self) -> f64 { self.y + self.h / 2.0 }

    pub fn union(&self, o: &BoundingBox) -> BoundingBox {
        let x = self.x.min(o.x); let y = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        BoundingBox { x, y, w: r - x, h: b - y }
    }
    pub fn x_overlap(&self, o: &BoundingBox) -> f64 {
        self.right().min(o.right()) - self.x.max(o.x)
    }
    pub fn y_gap(&self, o: &BoundingBox) -> f64 {
        self.y.max(o.y) - self.bottom().min(o.bottom())
    }
}

#[derive(Debug, Clone)]
pub struct TextLine  { pub text: String, pub bbox: BoundingBox }

#[derive(Debug, Clone)]
pub struct LayoutBlock { pub bbox: BoundingBox, pub lines: Vec<TextLine> }

#[derive(Debug, Clone)]
struct LayoutColumn { blocks: Vec<LayoutBlock> }

impl LayoutColumn {
    fn center_x(&self) -> f64 {
        if self.blocks.is_empty() { return 0.0; }
        self.blocks.iter().map(|b| b.bbox.center_x()).sum::<f64>() / self.blocks.len() as f64
    }
    fn bbox(&self) -> Option<BoundingBox> {
        let mut it = self.blocks.iter();
        let f = it.next()?.bbox.clone();
        Some(it.fold(f, |a, b| a.union(&b.bbox)))
    }
}

// ─── Геометрические алгоритмы (OCR-фолбек) ───────────────────────────────────

fn median_h(nodes: &[OCRNode]) -> f64 {
    if nodes.is_empty() { return 14.0; }
    let mut h: Vec<f64> = nodes.iter().map(|n| n.frame.h).collect();
    h.sort_by(|a, b| a.partial_cmp(b).unwrap());
    h[h.len() / 2]
}

fn merge_words_into_lines(nodes: &[OCRNode], mh: f64, cfg: &LayoutConfig) -> Vec<TextLine> {
    let mut sorted = nodes.to_vec();
    sorted.sort_by(|a, b| {
        if (a.frame.y - b.frame.y).abs() < mh * cfg.line_y_alignment_ratio {
            a.frame.x.partial_cmp(&b.frame.x).unwrap()
        } else {
            a.frame.y.partial_cmp(&b.frame.y).unwrap()
        }
    });

    let mut lines: Vec<TextLine> = Vec::new();
    for el in &sorted {
        let eb = BoundingBox { x: el.frame.x, y: el.frame.y, w: el.frame.w, h: el.frame.h };
        let best = lines.iter().enumerate().filter_map(|(i, l)| {
            let yd = (l.bbox.center_y() - eb.center_y()).abs();
            let xg = eb.x - l.bbox.right();
            if yd < mh * cfg.line_y_alignment_ratio
                && xg >= -(mh * cfg.word_merge_x_overlap_ratio)
                && xg <   (mh * cfg.word_merge_x_gap_ratio)
            { Some((i, xg)) } else { None }
        }).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|(i, _)| i);

        if let Some(idx) = best {
            let l = &mut lines[idx];
            if eb.x - l.bbox.right() > 0.5 { l.text.push(' '); }
            l.text.push_str(&el.text);
            l.bbox = l.bbox.union(&eb);
        } else {
            lines.push(TextLine { text: el.text.clone(), bbox: eb });
        }
    }
    lines
}

fn cluster_lines_into_blocks(lines: &[TextLine], mh: f64, cfg: &LayoutConfig) -> Vec<LayoutBlock> {
    let n = lines.len();
    if n == 0 { return vec![]; }

    let mut adj = vec![vec![]; n];
    for i in 0..n {
        for j in (i+1)..n {
            let a = &lines[i].bbox; let b = &lines[j].bbox;
            if a.y_gap(b) < mh * cfg.block_y_gap_ratio
                && (a.x_overlap(b) > mh * cfg.block_x_overlap_ratio
                || (a.x - b.x).abs() < mh * cfg.block_x_alignment_ratio)
            { adj[i].push(j); adj[j].push(i); }
        }
    }

    let mut visited = vec![false; n];
    let mut blocks  = Vec::new();
    for start in 0..n {
        if visited[start] { continue; }
        let mut comp = Vec::new();
        let mut q    = VecDeque::new();
        visited[start] = true; q.push_back(start);
        while let Some(cur) = q.pop_front() {
            comp.push(lines[cur].clone());
            for &nb in &adj[cur] {
                if !visited[nb] { visited[nb] = true; q.push_back(nb); }
            }
        }
        comp.sort_by(|a, b| a.bbox.y.partial_cmp(&b.bbox.y).unwrap());
        let bbox = comp.iter().skip(1).fold(comp[0].bbox.clone(), |a, l| a.union(&l.bbox));
        blocks.push(LayoutBlock { bbox, lines: comp });
    }
    blocks
}

fn split_into_columns(blocks: Vec<LayoutBlock>, mh: f64, cfg: &LayoutConfig) -> Vec<LayoutColumn> {
    if blocks.is_empty() { return vec![]; }
    let mut centers: Vec<f64> = blocks.iter().map(|b| b.bbox.center_x()).collect();
    centers.sort_by(|a, b| a.partial_cmp(b).unwrap());
    centers.dedup_by(|a, b| (*a - *b).abs() < 0.1);

    let thr = mh * cfg.column_gutter_ratio;
    let mut bounds = vec![f64::MIN];
    for w in centers.windows(2) {
        if w[1] - w[0] > thr { bounds.push((w[0] + w[1]) / 2.0); }
    }
    bounds.push(f64::MAX);

    let nc = bounds.len() - 1;
    let mut cols: Vec<LayoutColumn> = (0..nc).map(|_| LayoutColumn { blocks: vec![] }).collect();
    for block in blocks {
        let cx = block.bbox.center_x();
        let ci = bounds.windows(2).position(|w| cx > w[0] && cx <= w[1]).unwrap_or(nc - 1);
        cols[ci].blocks.push(block);
    }
    cols.retain(|c| !c.blocks.is_empty());
    cols
}

fn merge_small_columns(mut cols: Vec<LayoutColumn>, cfg: &LayoutConfig) -> Vec<LayoutColumn> {
    if cols.len() < 2 { return cols; }
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < cols.len() {
            if cols.len() < 2 { break; }
            if cols[i].blocks.len() < cfg.column_min_blocks {
                let cx = cols[i].center_x();
                let ni = (0..cols.len()).filter(|&j| j != i)
                    .min_by(|&a, &b| {
                        let da = (cols[a].center_x() - cx).abs();
                        let db = (cols[b].center_x() - cx).abs();
                        da.partial_cmp(&db).unwrap()
                    });
                if let Some(nidx) = ni {
                    let small = cols.remove(i);
                    let ti = if nidx > i { nidx - 1 } else { nidx };
                    cols[ti].blocks.extend(small.blocks);
                    cols[ti].blocks.sort_by(|a, b| a.bbox.y.partial_cmp(&b.bbox.y).unwrap());
                    changed = true;
                } else { i += 1; }
            } else { i += 1; }
        }
    }
    cols
}

// ─── XML утилиты ──────────────────────────────────────────────────────────────

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
        .replace('>', "&gt;").replace('"', "&quot;")
}

// ─── Стратегия А: AX Tree ────────────────────────────────────────────────────

fn is_interactive_role(role: &str) -> bool {
    matches!(role, "AXTextField" | "AXTextArea" | "AXComboBox" | "AXCheckBox" |
        "AXRadioButton" | "AXButton" | "AXSlider" | "AXPopUpButton" |
        "AXLink" | "AXSearchField")
}

fn is_node_useful(node: &AXNode) -> bool {
    if is_interactive_role(&node.role) { return true; }
    if node.title.as_ref().is_some_and(|s| !s.trim().is_empty()) { return true; }
    if node.value.as_ref().is_some_and(|s| !s.trim().is_empty()) { return true; }
    if node.description.as_ref().is_some_and(|s| !s.trim().is_empty()) { return true; }
    node.children.as_ref().map_or(false, |c| c.iter().any(is_node_useful))
}

fn count_useful_nodes(node: &AXNode) -> usize {
    let self_count = (node.title.as_ref().is_some_and(|s| !s.trim().is_empty())
        || node.value.as_ref().is_some_and(|s| !s.trim().is_empty())
        || node.description.as_ref().is_some_and(|s| !s.trim().is_empty())
        || is_interactive_role(&node.role)) as usize;

    self_count + node.children.as_ref().map_or(0, |c| c.iter().map(count_useful_nodes).sum())
}

fn get_useful_content_bounds(node: &AXNode) -> Option<BoundingBox> {
    let is_structural = matches!(node.role.as_str(),
        "AXWindow" | "AXGroup" | "AXScrollArea" | "AXSplitGroup" |
        "AXTabGroup" | "AXToolbar" | "AXApplication");

    let mut bounds: Option<BoundingBox> = None;

    if !is_structural {
        let has_content = node.title.as_ref().is_some_and(|s| !s.trim().is_empty())
            || node.value.as_ref().is_some_and(|s| !s.trim().is_empty())
            || node.description.as_ref().is_some_and(|s| !s.trim().is_empty());

        if has_content || is_interactive_role(&node.role) {
            if let Some(f) = &node.frame {
                bounds = Some(BoundingBox { x: f.x, y: f.y, w: f.w, h: f.h });
            }
        }
    }

    if let Some(children) = &node.children {
        for child in children {
            if let Some(cb) = get_useful_content_bounds(child) {
                bounds = Some(match bounds { Some(b) => b.union(&cb), None => cb });
            }
        }
    }
    bounds
}

fn serialize_ax_tree_recursive(node: &AXNode, indent: usize, xml: &mut String) {
    if !is_node_useful(node) { return; }
    let ind = " ".repeat(indent);
    xml.push_str(&format!("{}<Node role=\"{}\"", ind, escape_xml(&node.role)));
    if let Some(t) = &node.title      { if !t.trim().is_empty() { xml.push_str(&format!(" title=\"{}\"",       escape_xml(t))); } }
    if let Some(v) = &node.value      { if !v.trim().is_empty() { xml.push_str(&format!(" value=\"{}\"",       escape_xml(v))); } }
    if let Some(d) = &node.description{ if !d.trim().is_empty() { xml.push_str(&format!(" description=\"{}\"", escape_xml(d))); } }
    if let Some(f) = &node.frame      { xml.push_str(&format!(" x=\"{}\" y=\"{}\" w=\"{}\" h=\"{}\"", f.x as i32, f.y as i32, f.w as i32, f.h as i32)); }

    if let Some(children) = &node.children {
        let useful: Vec<_> = children.iter().filter(|c| is_node_useful(c)).collect();
        if !useful.is_empty() {
            xml.push_str(">\n");
            for c in useful { serialize_ax_tree_recursive(c, indent + 2, xml); }
            xml.push_str(&format!("{}</Node>\n", ind));
            return;
        }
    }
    xml.push_str(" />\n");
}

pub fn generate_ax_layout_xml(root: &AXNode, title: &str) -> String {
    let mut xml = format!(
        "## Screen layout (AX Tree):\n```xml\n<Screen source=\"ax\">\n  <Window title=\"{}\" />\n\n",
        escape_xml(title)
    );
    serialize_ax_tree_recursive(root, 2, &mut xml);
    xml.push_str("</Screen>\n```\n");
    xml
}

// ─── Стратегия Б: OCR Fallback ────────────────────────────────────────────────

pub fn generate_ocr_layout_xml(
    nodes:     Vec<OCRNode>,
    title:     &str,
    cfg:       &LayoutConfig,
    annotator: &dyn BlockAnnotator,
) -> String {
    if nodes.is_empty() {
        return format!(
            "```xml\n<Screen source=\"ocr\">\n  <Window title=\"{}\" />\n  <!-- No OCR data -->\n</Screen>\n```\n",
            escape_xml(title)
        );
    }

    let mh      = median_h(&nodes);
    let lines   = merge_words_into_lines(&nodes, mh, cfg);
    let blocks  = cluster_lines_into_blocks(&lines, mh, cfg);
    let cols    = split_into_columns(blocks, mh, cfg);
    let cols    = merge_small_columns(cols, cfg);

    let mut xml = format!(
        "## Screen layout (OCR Fallback):\n```xml\n<Screen source=\"ocr\" median_font_h=\"{}\">\n  <Window title=\"{}\" />\n\n",
        mh as i32, escape_xml(title)
    );

    for (ci, col) in cols.iter().enumerate() {
        let cb = match col.bbox() { Some(b) => b, None => continue };
        xml.push_str(&format!(
            "  <Column id=\"{}\" x=\"{}\" y=\"{}\" w=\"{}\" h=\"{}\">\n",
            ci + 1, cb.x as i32, cb.y as i32, cb.w as i32, cb.h as i32
        ));

        let mut sorted = col.blocks.clone();
        sorted.sort_by(|a, b| a.bbox.y.partial_cmp(&b.bbox.y).unwrap());

        for block in &sorted {
            let ba = annotator.annotate_block(block);
            xml.push_str(&format!(
                "    <Block x=\"{}\" y=\"{}\" w=\"{}\" h=\"{}\"{}>\n",
                block.bbox.x as i32, block.bbox.y as i32,
                block.bbox.w as i32, block.bbox.h as i32,
                if ba.is_empty() { String::new() } else { format!(" {}", ba) }
            ));
            for line in &block.lines {
                let la = annotator.annotate_line(line);
                xml.push_str(&format!(
                    "      <Line x=\"{}\" y=\"{}\" h=\"{}\"{}>{}</Line>\n",
                    line.bbox.x as i32, line.bbox.y as i32, line.bbox.h as i32,
                    if la.is_empty() { String::new() } else { format!(" {}", la) },
                    escape_xml(&line.text)
                ));
            }
            xml.push_str("    </Block>\n");
        }
        xml.push_str("  </Column>\n\n");
    }
    xml.push_str("</Screen>\n```\n");
    xml
}

// ─── Публичный API ────────────────────────────────────────────────────────────

/// Быстрый вызов с дефолтными настройками.
pub fn build_layout_from_dump(json_dump: &str) -> String {
    build_layout_from_dump_with(
        json_dump,
        &LayoutConfig::default(),
        5,
        0.35,
        &NoopAnnotator,
    )
}

/// Полный вызов с явными параметрами из конфига.
pub fn build_layout_from_dump_with(
    json_dump:           &str,
    layout_cfg:          &LayoutConfig,
    ax_min_useful_nodes: usize,
    ax_min_coverage:     f64,
    annotator:           &dyn BlockAnnotator,
) -> String {
    let dump: DumpOutput = match serde_json::from_str(json_dump) {
        Ok(d)  => d,
        Err(e) => return format!("Parse error: {}", e),
    };

    let title = dump.ax_tree.as_ref()
        .and_then(|t| t.title.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    if let Some(ax_tree) = &dump.ax_tree {
        let useful_nodes = count_useful_nodes(ax_tree);
        let window_area  = ax_tree.frame.as_ref().map(|f| f.w * f.h).unwrap_or(0.0);

        let coverage_ok = if window_area > 0.0 {
            get_useful_content_bounds(ax_tree)
                .map(|cb| cb.w * cb.h / window_area >= ax_min_coverage)
                .unwrap_or(false)
        } else { true };

        if useful_nodes >= ax_min_useful_nodes && coverage_ok {
            return generate_ax_layout_xml(ax_tree, &title);
        }
    }

    generate_ocr_layout_xml(dump.ocr_text, &title, layout_cfg, annotator)
}