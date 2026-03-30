pub struct EditorState {
    pub textarea: tui_textarea::TextArea<'static>,
    pub entry_index: usize,
    pub original_content: String,
}
