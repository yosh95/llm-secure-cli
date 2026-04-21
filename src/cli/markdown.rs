use colored::*;
use comfy_table::Table;
use pulldown_cmark::{Alignment, Event, Options as CmarkOptions, Parser, Tag, TagEnd};
use std::fmt::Write;
use textwrap::{fill, Options as WrapOptions};

pub struct MarkdownRenderer {
    width: usize,
}

impl MarkdownRenderer {
    pub fn new(width: usize) -> Self {
        Self { width }
    }

    pub fn render(&self, markdown: &str) -> String {
        let mut options = CmarkOptions::empty();
        options.insert(CmarkOptions::ENABLE_TABLES);
        options.insert(CmarkOptions::ENABLE_STRIKETHROUGH);
        options.insert(CmarkOptions::ENABLE_TASKLISTS);

        let parser = Parser::new_ext(markdown, options);
        let mut output = String::new();
        let mut list_depth = 0;
        let mut in_table = false;
        let mut in_code_block = false;
        let mut table_headers = Vec::new();
        let mut table_rows = Vec::new();
        let mut table_alignments = Vec::new();
        let mut current_row = Vec::new();
        let mut in_table_head = false;
        let mut current_paragraph = String::new();
        let mut current_heading_level = None;

        let wrap_options = WrapOptions::new(self.width.saturating_sub(4)).break_words(false);

        for event in parser {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        self.flush_paragraph(
                            &mut output,
                            &mut current_paragraph,
                            &wrap_options,
                            0,
                            None,
                        );
                        let level_num = level as usize;
                        output.push('\n');
                        current_paragraph.push_str(&"#".repeat(level_num));
                        current_paragraph.push(' ');
                        current_heading_level = Some(level_num);
                    }
                    Tag::Paragraph => {
                        self.flush_paragraph(
                            &mut output,
                            &mut current_paragraph,
                            &wrap_options,
                            0,
                            None,
                        );
                    }
                    Tag::List(_) => {
                        self.flush_paragraph(
                            &mut output,
                            &mut current_paragraph,
                            &wrap_options,
                            0,
                            None,
                        );
                        list_depth += 1;
                    }
                    Tag::Item => {
                        self.flush_paragraph(
                            &mut output,
                            &mut current_paragraph,
                            &wrap_options,
                            list_depth * 2,
                            None,
                        );
                        output.push('\n');
                        output.push_str(&"  ".repeat(list_depth - 1));
                        output.push_str("• ");
                    }
                    Tag::Table(alignments) => {
                        self.flush_paragraph(
                            &mut output,
                            &mut current_paragraph,
                            &wrap_options,
                            0,
                            None,
                        );
                        in_table = true;
                        table_alignments = alignments;
                        table_headers.clear();
                        table_rows.clear();
                        output.push('\n');
                    }
                    Tag::TableHead => {
                        in_table_head = true;
                        current_row.clear();
                    }
                    Tag::TableRow => {
                        current_row.clear();
                    }
                    Tag::TableCell => {}
                    Tag::CodeBlock(kind) => {
                        self.flush_paragraph(
                            &mut output,
                            &mut current_paragraph,
                            &wrap_options,
                            0,
                            None,
                        );
                        output.push_str("\n\n```");
                        if let pulldown_cmark::CodeBlockKind::Fenced(lang) = kind {
                            output.push_str(&lang);
                        }
                        output.push('\n');
                        in_code_block = true;
                    }
                    _ => {}
                },
                Event::End(tag) => {
                    match tag {
                        TagEnd::Heading(_) => {
                            self.flush_paragraph(
                                &mut output,
                                &mut current_paragraph,
                                &wrap_options,
                                0,
                                current_heading_level,
                            );
                            output.push('\n');
                            current_heading_level = None;
                        }
                        TagEnd::Item => {
                            let indent = if list_depth > 0 { list_depth * 2 } else { 0 };
                            self.flush_paragraph(
                                &mut output,
                                &mut current_paragraph,
                                &wrap_options,
                                indent,
                                None,
                            );
                        }
                        TagEnd::Paragraph => {
                            let indent = if list_depth > 0 { list_depth * 2 } else { 0 };
                            self.flush_paragraph(
                                &mut output,
                                &mut current_paragraph,
                                &wrap_options,
                                indent,
                                None,
                            );
                            if !output.ends_with('\n') {
                                output.push('\n');
                            }
                        }
                        TagEnd::List(_) => {
                            self.flush_paragraph(
                                &mut output,
                                &mut current_paragraph,
                                &wrap_options,
                                0,
                                None,
                            );
                            list_depth -= 1;
                            if !output.is_empty() && !output.ends_with('\n') {
                                output.push('\n');
                            }
                        }
                        TagEnd::Table => {
                            in_table = false;
                            let mut table = Table::new();
                            table.load_preset(comfy_table::presets::UTF8_FULL);
                            table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
                            table.set_width(self.width as u16);

                            if !table_headers.is_empty() {
                                table.set_header(&table_headers);
                            }

                            for (i, align) in table_alignments.iter().enumerate() {
                                if let Some(column) = table.column_mut(i) {
                                    match align {
                                        Alignment::Left => column
                                            .set_cell_alignment(comfy_table::CellAlignment::Left),
                                        Alignment::Center => column
                                            .set_cell_alignment(comfy_table::CellAlignment::Center),
                                        Alignment::Right => column
                                            .set_cell_alignment(comfy_table::CellAlignment::Right),
                                        Alignment::None => {}
                                    }
                                }
                            }

                            for row in table_rows.drain(..) {
                                table.add_row(row);
                            }

                            output.push_str(&table.to_string());
                            output.push('\n');
                        }
                        TagEnd::TableHead => {
                            in_table_head = false;
                            table_headers = std::mem::take(&mut current_row);
                        }
                        TagEnd::TableRow if !in_table_head => {
                            table_rows.push(std::mem::take(&mut current_row));
                        }
                        TagEnd::CodeBlock => {
                            in_code_block = false;
                            if !output.ends_with('\n') {
                                output.push('\n');
                            }
                            output.push_str("```\n");
                        }
                        _ => {}
                    }
                }
                Event::Text(text) => {
                    if in_table {
                        current_row.push(text.to_string());
                    } else if in_code_block {
                        output.push_str(&text);
                    } else {
                        current_paragraph.push_str(&text);
                    }
                }
                Event::Code(text) => {
                    if in_table {
                        current_row.push(format!("`{}`", text));
                    } else {
                        let _ = write!(current_paragraph, "`{}`", text);
                    }
                }
                Event::SoftBreak if !in_table => {
                    current_paragraph.push(' ');
                }
                Event::HardBreak if !in_table => {
                    current_paragraph.push('\n');
                }
                Event::Rule => {
                    self.flush_paragraph(
                        &mut output,
                        &mut current_paragraph,
                        &wrap_options,
                        0,
                        None,
                    );
                    output.push_str("\n---\n");
                }
                _ => {}
            }
        }

        self.flush_paragraph(&mut output, &mut current_paragraph, &wrap_options, 0, None);
        output.trim().to_string()
    }

    fn flush_paragraph(
        &self,
        output: &mut String,
        paragraph: &mut String,
        options: &WrapOptions,
        indent_size: usize,
        heading_level: Option<usize>,
    ) {
        if paragraph.is_empty() {
            return;
        }

        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            paragraph.clear();
            return;
        }

        let filled = fill(trimmed, options);
        let indent = " ".repeat(indent_size);

        for (i, line) in filled.lines().enumerate() {
            if i > 0 || output.ends_with('\n') {
                output.push('\n');
                output.push_str(&indent);
            } else if output.is_empty() {
                output.push_str(&indent);
            }

            if let Some(level) = heading_level {
                let colored_line = match level {
                    1 => line.cyan().bold(),
                    2 => line.yellow().bold(),
                    3 => line.green().bold(),
                    _ => line.bright_black().bold(),
                };
                output.push_str(&colored_line.to_string());
            } else {
                output.push_str(line);
            }
        }

        paragraph.clear();
    }
}

pub fn render_markdown(content: &str, width: usize) -> String {
    let renderer = MarkdownRenderer::new(width);
    renderer.render(content)
}
