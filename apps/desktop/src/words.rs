//! The few texts the app shows itself rather than through the window: the titles of file
//! dialogs and of system notifications, in the languages the window is translated into
//! (`ui/src/i18n`). Engine errors stay in English everywhere.

/// A text the app shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    /// The title of the dialog that opens a project.
    OpenProject,
    /// The title of the dialog that saves a new project.
    CreateProject,
    /// The title of the dialog that relinks a missing file.
    FindMedia,
    /// The title of the dialog that chooses an export's file.
    ExportTo,
    /// The title of the dialog that chooses a folder for a preference.
    ChooseFolder,
    /// A notification's title: an export succeeded.
    ExportFinished,
    /// A notification's title: an export failed.
    ExportFailed,
    /// A notification's title: a recording succeeded.
    RecordingFinished,
    /// A notification's title: a recording failed.
    RecordingFailed,
    /// The app menu's item that opens Settings.
    Settings,
    /// The title of the dialog that imports media files.
    ImportMedia,
    /// The import dialog's choice of every kind of media file.
    MediaFiles,
    /// The import dialog's choice of video files.
    VideoFiles,
    /// The import dialog's choice of sound files.
    AudioFiles,
    /// The import dialog's choice of image files.
    ImageFiles,
    /// The import dialog's choice of vector drawings (`.svg`).
    DrawingFiles,
    /// The import dialog's choice of web composition pages (`.html`).
    WebFiles,
    /// The import dialog's choice of 3D models (`.glb`, `.gltf`).
    ModelFiles,
    /// The import dialog's choice of color lookup tables.
    LutFiles,
    /// The import dialog's choice of font files.
    FontFiles,
    /// The import dialog's choice of any file.
    AllFiles,
    /// The title of the dialog that chooses a web composition's page or folder.
    ImportComposition,
}

/// `word` in `language` (a code of `fosscut_engine::preferences::LANGUAGES`); English for
/// any other.
pub fn word(language: &str, word: Word) -> &'static str {
    match (language, word) {
        ("hu", Word::OpenProject) => "FossCut-projekt megnyitása",
        ("hu", Word::CreateProject) => "Új FossCut-projekt létrehozása",
        ("hu", Word::FindMedia) => "A médiafájl megkeresése",
        ("hu", Word::ExportTo) => "Exportálás ide",
        ("hu", Word::ChooseFolder) => "Mappa kiválasztása",
        ("hu", Word::ExportFinished) => "Az exportálás elkészült",
        ("hu", Word::ExportFailed) => "Az exportálás nem sikerült",
        ("hu", Word::RecordingFinished) => "A felvétel elkészült",
        ("hu", Word::RecordingFailed) => "A felvétel nem sikerült",
        ("hu", Word::Settings) => "Beállítások\u{2026}",
        ("hu", Word::ImportMedia) => "Médiafájlok importálása",
        ("hu", Word::MediaFiles) => "Médiafájlok",
        ("hu", Word::VideoFiles) => "Videók",
        ("hu", Word::AudioFiles) => "Hangfájlok",
        ("hu", Word::ImageFiles) => "Képek",
        ("hu", Word::DrawingFiles) => "Vektorrajzok (.svg)",
        ("hu", Word::WebFiles) => "Webes kompozíciók (.html)",
        ("hu", Word::ModelFiles) => "3D-modellek (.glb, .gltf)",
        ("hu", Word::LutFiles) => "LUT-ok (.cube)",
        ("hu", Word::FontFiles) => "Betűtípusok",
        ("hu", Word::AllFiles) => "Minden fájl",
        ("hu", Word::ImportComposition) => "Webes kompozíció importálása",
        (_, Word::OpenProject) => "Open a FossCut project",
        (_, Word::CreateProject) => "Create a FossCut project",
        (_, Word::FindMedia) => "Find the media file",
        (_, Word::ExportTo) => "Export to",
        (_, Word::ChooseFolder) => "Choose a folder",
        (_, Word::ExportFinished) => "Export finished",
        (_, Word::ExportFailed) => "Export failed",
        (_, Word::RecordingFinished) => "Recording finished",
        (_, Word::RecordingFailed) => "Recording failed",
        (_, Word::Settings) => "Settings\u{2026}",
        (_, Word::ImportMedia) => "Import media",
        (_, Word::MediaFiles) => "Media files",
        (_, Word::VideoFiles) => "Videos",
        (_, Word::AudioFiles) => "Audio",
        (_, Word::ImageFiles) => "Images",
        (_, Word::DrawingFiles) => "Vector drawings (.svg)",
        (_, Word::WebFiles) => "Web compositions (.html)",
        (_, Word::ModelFiles) => "3D models (.glb, .gltf)",
        (_, Word::LutFiles) => "LUTs (.cube)",
        (_, Word::FontFiles) => "Fonts",
        (_, Word::AllFiles) => "All files",
        (_, Word::ImportComposition) => "Import a web composition",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_is_translated_and_other_languages_read_english() {
        let all = [
            Word::OpenProject,
            Word::CreateProject,
            Word::FindMedia,
            Word::ExportTo,
            Word::ChooseFolder,
            Word::ExportFinished,
            Word::ExportFailed,
            Word::RecordingFinished,
            Word::RecordingFailed,
            Word::Settings,
            Word::ImportMedia,
            Word::MediaFiles,
            Word::VideoFiles,
            Word::AudioFiles,
            Word::ImageFiles,
            Word::DrawingFiles,
            Word::WebFiles,
            Word::ModelFiles,
            Word::LutFiles,
            Word::FontFiles,
            Word::AllFiles,
            Word::ImportComposition,
        ];
        for text in all {
            assert_ne!(word("hu", text), word("en", text), "{text:?}");
            assert_eq!(word("xx", text), word("en", text), "{text:?}");
        }
    }
}
