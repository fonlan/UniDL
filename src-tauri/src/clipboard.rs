pub fn read_text() -> Result<Option<String>, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| clipboard_error_message(&error))?;
    match clipboard.get_text() {
        Ok(text) => Ok(Some(text)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(clipboard_error_message(&error)),
    }
}

fn clipboard_error_message(error: &arboard::Error) -> String {
    format!("failed to read clipboard text: {error}")
}
