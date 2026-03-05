use serde::Deserialize;
use std::collections::VecDeque;

// ─── Модели данных (соответствуют JSON из Swift) ──────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct FrameData {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Deserialize)]
pub struct AXNode {
    pub role: String,
    pub title: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub frame: Option<FrameData>,
    pub children: Option<Vec<AXNode>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OCRNode {
    pub text: String,
    pub frame: FrameData,
}

#[derive(Debug, Deserialize)]
pub struct DumpOutput {
    #[serde(rename = "axTree")]
    pub ax_tree: Option<AXNode>,
    #[serde(rename = "ocrText")]
    pub ocr_text: Vec<OCRNode>,
}

// ─── Внутренние структуры для кластеризации ───────────────────────────────────

struct Block {
    nodes: Vec<OCRNode>,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Block {
    fn new(nodes: Vec<OCRNode>) -> Self {
        let min_x = nodes.iter().map(|n| n.frame.x).fold(f64::MAX, f64::min);
        let min_y = nodes.iter().map(|n| n.frame.y).fold(f64::MAX, f64::min);
        let max_x = nodes.iter().map(|n| n.frame.x + n.frame.w).fold(f64::MIN, f64::max);
        let max_y = nodes.iter().map(|n| n.frame.y + n.frame.h).fold(f64::MIN, f64::max);
        Self { nodes, min_x, min_y, max_x, max_y }
    }
}

// ─── Строгий Графовый Алгоритм (Strict Proximity Clustering) ──────────────────

pub fn process_dump_to_markdown(json_str: &str) -> String {
    let dump: DumpOutput = match serde_json::from_str(json_str) {
        Ok(d) => d,
        Err(e) => return format!("Ошибка парсинга JSON: {}", e),
    };

    let mut md = String::new();

    if let Some(ax) = dump.ax_tree {
        let title = ax.title.unwrap_or_else(|| "Unknown Window".to_string());
        md.push_str(&format!("# [Window: {}]\n\n", title));
    }

    if !dump.ocr_text.is_empty() {
        md.push_str(&build_layout_from_ocr(dump.ocr_text));
    } else {
        md.push_str("*(Экран пуст или текст не найден)*\n");
    }

    md
}

fn build_layout_from_ocr(nodes: Vec<OCRNode>) -> String {
    let n = nodes.len();
    if n == 0 { return String::new(); }

    // 1. Динамический расчет масштаба интерфейса (медианная высота шрифта)
    let mut heights: Vec<f64> = nodes.iter().map(|n| n.frame.h).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_h = heights[heights.len() / 2];

    let screen_w = nodes.iter().map(|n| n.frame.x + n.frame.w).fold(0.0, f64::max);

    // 2. Строим граф связей (Матрица смежности)
    let mut adj = vec![vec![]; n];

    for i in 0..n {
        for j in (i + 1)..n {
            let a = &nodes[i].frame;
            let b = &nodes[j].frame;

            let a_right = a.x + a.w;
            let a_bottom = a.y + a.h;
            let b_right = b.x + b.w;
            let b_bottom = b.y + b.h;

            let x_overlap = (a_right.min(b_right) - a.x.max(b.x)).max(0.0);
            let y_overlap = (a_bottom.min(b_bottom) - a.y.max(b.y)).max(0.0);
            let x_gap = (a.x.max(b.x) - a_right.min(b_right)).max(0.0);
            let y_gap = (a.y.max(b.y) - a_bottom.min(b_bottom)).max(0.0);

            let avg_h = (a.h + b.h) / 2.0;
            let mut connected = false;

            // ПРАВИЛО А: Горизонтальная связь (Слова в одной строке)
            // Жесткое ограничение: x_gap < avg_h * 1.5. Это не позволит сайдбару слипнуться с чатом!
            if y_overlap > avg_h * 0.4 && x_gap < avg_h * 1.5 {
                connected = true;
            }

            // ПРАВИЛО Б: Вертикальная связь (Строки одного сообщения/абзаца)
            // Жесткое ограничение: сильное пересечение по X, чтобы элементы из разных колонок не слипались
            if y_gap < avg_h * 1.5 && x_overlap > avg_h * 0.5 {
                connected = true;
            }

            if connected {
                adj[i].push(j);
                adj[j].push(i);
            }
        }
    }

    // 3. Извлекаем связные компоненты (наши чистые изолированные Блоки) через BFS
    let mut visited = vec![false; n];
    let mut blocks = Vec::new();

    for i in 0..n {
        if !visited[i] {
            let mut comp_nodes = Vec::new();
            let mut q = VecDeque::new();

            visited[i] = true;
            q.push_back(i);

            while let Some(curr) = q.pop_front() {
                comp_nodes.push(nodes[curr].clone());
                for &neighbor in &adj[curr] {
                    if !visited[neighbor] {
                        visited[neighbor] = true;
                        q.push_back(neighbor);
                    }
                }
            }
            blocks.push(Block::new(comp_nodes));
        }
    }

    // 4. Естественная визуальная сортировка блоков (как чтение книги: сверху-вниз, слева-направо)
    blocks.sort_by(|a, b| {
        if (a.min_y - b.min_y).abs() < median_h * 1.0 {
            a.min_x.partial_cmp(&b.min_x).unwrap()
        } else {
            a.min_y.partial_cmp(&b.min_y).unwrap()
        }
    });

    // 5. Рендеринг финального Markdown
    let mut output = String::new();

    for block in blocks {
        // Умная семантика для LLM: понимаем левое меню, входящие и исходящие сообщения
        let center_x = block.min_x + (block.max_x - block.min_x) / 2.0;
        let align = if center_x < screen_w * 0.35 {
            "Sidebar/Left"
        } else if center_x > screen_w * 0.65 {
            "Right (Outgoing/Self)"
        } else {
            "Center (Incoming/Other)"
        };

        output.push_str(&format!(
            "## [Block at X:{}, Y:{}] - {}\n",
            block.min_x as i32, block.min_y as i32,
            align
        ));

        // Внутри блока сортируем и форматируем текст по строкам
        let mut b_nodes = block.nodes;
        b_nodes.sort_by(|a, b| {
            if (a.frame.y - b.frame.y).abs() <= median_h * 0.5 {
                a.frame.x.partial_cmp(&b.frame.x).unwrap()
            } else {
                a.frame.y.partial_cmp(&b.frame.y).unwrap()
            }
        });

        let mut current_y = b_nodes[0].frame.y;
        let mut row_text = String::new();
        let mut last_x = 0.0;

        for node in b_nodes {
            if (node.frame.y - current_y).abs() > median_h * 0.6 {
                output.push_str(&row_text);
                output.push('\n');
                row_text.clear();
                current_y = node.frame.y;
                last_x = 0.0;
            }

            if !row_text.is_empty() {
                if (node.frame.x - last_x) > median_h * 2.0 {
                    row_text.push_str(" | ");
                } else {
                    row_text.push(' ');
                }
            }
            row_text.push_str(&node.text);
            last_x = node.frame.x + node.frame.w;
        }
        if !row_text.is_empty() {
            output.push_str(&row_text);
            output.push('\n');
        }
        output.push('\n');
    }

    output
}