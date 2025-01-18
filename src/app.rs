use std::{num::NonZeroUsize, path::PathBuf};

use crate::{
    canvas::{Canvas, Token, TokenPosition, TokenStyle},
    git::{GrepOptions, MatchLine, SearchResult},
    terminal::Terminal,
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use orfail::OrFail;

#[derive(Debug)]
pub struct App {
    terminal: Terminal,
    exit: bool,
    frame_row_start: usize,
    state: AppState,
    widgets: Vec<Box<dyn 'static + Widget>>,
}

impl App {
    pub fn new() -> orfail::Result<Self> {
        let terminal = Terminal::new().or_fail()?;
        Ok(Self {
            terminal,
            exit: false,
            frame_row_start: 0,
            state: AppState::default(),
            widgets: vec![Box::new(MainWidget {
                tree: Tree::default(),
                cursor: Cursor::default(),
            })],
        })
    }

    pub fn run(mut self) -> orfail::Result<()> {
        self.render().or_fail()?;

        while !self.exit {
            let event = self.terminal.next_event().or_fail()?;
            self.handle_event(event).or_fail()?;
        }

        std::mem::drop(self.terminal);
        println!("{}", self.state.grep.command_string());

        Ok(())
    }

    fn render(&mut self) -> orfail::Result<()> {
        if self.terminal.size().is_empty() {
            return Ok(());
        }

        let mut canvas = Canvas::new(self.frame_row_start, self.terminal.size());
        for widget in &self.widgets {
            widget.render(&self.state, &mut canvas).or_fail()?;
        }
        if let Some(widget) = self.widgets.last() {
            widget.render_legend(&mut canvas).or_fail()?;
        }
        self.terminal.draw_frame(canvas.into_frame()).or_fail()?;

        self.state.dirty = false;
        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> orfail::Result<()> {
        match event {
            Event::FocusGained => Ok(()),
            Event::FocusLost => Ok(()),
            Event::Key(event) => self.handle_key_event(event).or_fail(),
            Event::Mouse(_) => Ok(()),
            Event::Paste(_) => Ok(()),
            Event::Resize(_, _) => self.render().or_fail(),
        }
    }

    fn handle_key_event(&mut self, event: KeyEvent) -> orfail::Result<()> {
        if event.kind != KeyEventKind::Press {
            return Ok(());
        }

        let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
        match event.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.exit = true;
            }
            KeyCode::Char('c') if ctrl => {
                self.exit = true;
            }
            _ => {
                if let Some(widget) = self.widgets.last_mut() {
                    if !widget.handle_key_event(&mut self.state, event).or_fail()? {
                        self.widgets.pop();
                        self.state.dirty = true;
                    }
                    if let Some(widget) = self.state.new_widget.take() {
                        self.widgets.push(widget);
                        self.state.dirty = true;
                    }
                }
            }
        }

        if self.state.dirty {
            self.render().or_fail()?;
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct AppState {
    grep: GrepOptions,
    new_widget: Option<Box<dyn 'static + Widget>>,
    dirty: bool,
    search_result: SearchResult,
}

impl AppState {
    pub fn regrep(&mut self) -> orfail::Result<()> {
        self.search_result = self.grep.call().or_fail()?;
        self.dirty = true;
        Ok(())
    }
}

pub trait Widget: std::fmt::Debug {
    fn render(&self, state: &AppState, canvas: &mut Canvas) -> orfail::Result<()>;
    fn render_legend(&self, canvas: &mut Canvas) -> orfail::Result<()>;
    fn handle_key_event(&mut self, state: &mut AppState, event: KeyEvent) -> orfail::Result<bool>;
}

#[derive(Debug)]
pub struct MainWidget {
    pub tree: Tree,
    pub cursor: Cursor,
}

impl Widget for MainWidget {
    fn render(&self, state: &AppState, canvas: &mut Canvas) -> orfail::Result<()> {
        canvas.drawln(Token::new(state.grep.command_string()));
        canvas.drawln(Token::new(
            std::iter::repeat_n('-', canvas.frame_size().cols).collect::<String>(),
        ));

        self.tree.render(canvas, &self.cursor, &state.search_result);

        Ok(())
    }

    fn render_legend(&self, _canvas: &mut Canvas) -> orfail::Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, state: &mut AppState, event: KeyEvent) -> orfail::Result<bool> {
        match event.code {
            KeyCode::Char('/') => {
                state.new_widget = Some(Box::new(SearchPatternInputWidget {}));
            }
            KeyCode::Char('a') => {
                if state.grep.after_context == 0 {
                    state.grep.after_context = 3;
                } else {
                    state.grep.after_context = 0;
                }
                state.regrep().or_fail()?;
            }
            KeyCode::Char('b') => {
                if state.grep.before_context == 0 {
                    state.grep.before_context = 3;
                } else {
                    state.grep.before_context = 0;
                }
                state.regrep().or_fail()?;
            }
            KeyCode::Char('i') => {
                state.grep.ignore_case = !state.grep.ignore_case;
                state.regrep().or_fail()?;
            }

            _ => {}
        }
        Ok(true)
    }
}

#[derive(Debug, Default)]
pub struct Tree {}

impl Tree {
    fn render(&self, canvas: &mut Canvas, cursor: &Cursor, result: &SearchResult) {
        for (file, lines) in &result.files {
            let hits = result
                .highlight
                .lines
                .get(file)
                .map(|v| v.values().map(|v| v.len()).sum::<usize>())
                .unwrap_or_default();
            canvas.draw(Token::with_style(
                format!("{}", file.display()),
                TokenStyle::Underlined,
            ));
            canvas.drawln(Token::new(format!(" ({} lines, {hits} hits)", lines.len())));
            self.render_lines(canvas, cursor, result, file, lines);
        }
    }

    fn render_lines(
        &self,
        canvas: &mut Canvas,
        _cursor: &Cursor,
        result: &SearchResult,
        file: &PathBuf,
        lines: &[MatchLine],
    ) {
        for line in lines {
            // TODO:
            let matched_columns = result
                .highlight
                .lines
                .get(file)
                .and_then(|v| v.get(&line.number))
                .map(|v| v.as_slice())
                .unwrap_or(&[]);

            canvas.draw(Token::new(format!(
                "  [{:>width$}]",
                line.number,
                width = result.max_line_width
            )));

            let base = canvas.cursor();
            canvas.draw(Token::new(format!("{}", line.text)));

            for matched in matched_columns {
                let s = line
                    .text
                    .chars()
                    .skip(matched.column_offset)
                    .take(matched.text_chars)
                    .collect::<String>();
                canvas.draw_at(
                    TokenPosition {
                        row: base.row,
                        // TODO: Consider multi byte char
                        col: base.col + matched.column_offset,
                    },
                    Token::with_style(s, TokenStyle::Reverse),
                );
            }
            // TODO: optimize

            canvas.newline();
        }
    }
}

#[derive(Debug, Default)]
pub struct Cursor {
    pub file: Option<PathBuf>,
    pub line_number: Option<NonZeroUsize>,
}

#[derive(Debug)]
pub struct SearchPatternInputWidget {}

impl Widget for SearchPatternInputWidget {
    fn render(&self, _state: &AppState, canvas: &mut Canvas) -> orfail::Result<()> {
        canvas.drawln(Token::new("Grep: "));
        Ok(())
    }

    fn render_legend(&self, _canvas: &mut Canvas) -> orfail::Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, state: &mut AppState, event: KeyEvent) -> orfail::Result<bool> {
        match event.code {
            KeyCode::Enter => {
                state.search_result = state.grep.call().or_fail()?;
                state.dirty = true;
                return Ok(false);
            }
            KeyCode::Char(c) if !c.is_control() => {
                state.grep.pattern.push(c);
                state.dirty = true;
            }
            KeyCode::Backspace => {
                state.grep.pattern.pop();
                state.dirty = true;
            }
            _ => {}
        }
        Ok(true)
    }
}
