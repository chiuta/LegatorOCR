//! Minimal i18n: a static table (key → text in each of the 8 supported languages).
//! Missing keys show the key itself, so a gap is visible rather than blank.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ro,
    En,
    Fr,
    It,
    Es,
    Pt,
    De,
    Hu,
}

impl Lang {
    /// Column order of [`TABLE`].
    pub const ALL: [(Lang, &'static str, &'static str); 8] = [
        (Lang::Ro, "ro", "Română"),
        (Lang::En, "en", "English"),
        (Lang::Fr, "fr", "Français"),
        (Lang::It, "it", "Italiano"),
        (Lang::Es, "es", "Español"),
        (Lang::Pt, "pt", "Português"),
        (Lang::De, "de", "Deutsch"),
        (Lang::Hu, "hu", "Magyar"),
    ];

    pub fn code(self) -> &'static str {
        Self::ALL[self.index()].1
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|(l, _, _)| *l == self).unwrap_or(1)
    }

    /// `ro`, `pt-BR`, `de_AT.UTF-8` … → the matching language.
    pub fn from_code(code: &str) -> Option<Lang> {
        let c = code.trim().to_ascii_lowercase();
        let primary = c.split(['-', '_', '.']).next().unwrap_or("");
        Self::ALL.iter().find(|(_, k, _)| *k == primary).map(|(l, _, _)| *l)
    }

    /// System language, English when unsupported.
    pub fn detect() -> Self {
        sys_locale::get_locale().and_then(|l| Lang::from_code(&l)).unwrap_or(Lang::En)
    }
}

pub fn t(lang: Lang, key: &'static str) -> &'static str {
    TABLE.iter().find(|(k, _)| *k == key).map(|(_, v)| v[lang.index()]).unwrap_or(key)
}

const TABLE: &[(&str, [&str; 8])] = &[
    ("menu.file", ["Fișier", "File", "Fichier", "File", "Archivo", "Ficheiro", "Datei", "Fájl"]),
    ("menu.add_images", ["Adaugă imagini…", "Add images…", "Ajouter des images…", "Aggiungi immagini…", "Añadir imágenes…", "Adicionar imagens…", "Bilder hinzufügen…", "Képek hozzáadása…"]),
    ("menu.add_folder", ["Adaugă un folder…", "Add folder…", "Ajouter un dossier…", "Aggiungi cartella…", "Añadir carpeta…", "Adicionar pasta…", "Ordner hinzufügen…", "Mappa hozzáadása…"]),
    ("menu.export_pdf", ["Exportă PDF…", "Export PDF…", "Exporter en PDF…", "Esporta PDF…", "Exportar PDF…", "Exportar PDF…", "PDF exportieren…", "PDF exportálása…"]),
    ("menu.export_images", ["Exportă imaginile…", "Export images…", "Exporter les images…", "Esporta immagini…", "Exportar imágenes…", "Exportar imagens…", "Bilder exportieren…", "Képek exportálása…"]),
    ("menu.quit", ["Ieșire", "Quit", "Quitter", "Esci", "Salir", "Sair", "Beenden", "Kilépés"]),
    ("menu.language", ["Limbă", "Language", "Langue", "Lingua", "Idioma", "Idioma", "Sprache", "Nyelv"]),
    ("menu.about", ["Despre", "About", "À propos", "Informazioni", "Acerca de", "Sobre", "Über", "Névjegy"]),
    ("about.text", ["Post-procesare de scanări: separare pagini, îndreptare, decupare, curățare, PDF cu OCR. Implementare clean-room în Rust.", "Scan post-processing: page split, deskew, crop, clean-up, PDF with OCR. Clean-room implementation in Rust.", "Post-traitement de numérisations : séparation des pages, redressement, recadrage, nettoyage, PDF avec OCR. Implémentation « clean-room » en Rust.", "Post-elaborazione di scansioni: divisione pagine, raddrizzamento, ritaglio, pulizia, PDF con OCR. Implementazione clean-room in Rust.", "Posprocesado de escaneos: separación de páginas, enderezado, recorte, limpieza, PDF con OCR. Implementación clean-room en Rust.", "Pós-processamento de digitalizações: separação de páginas, endireitamento, recorte, limpeza, PDF com OCR. Implementação clean-room em Rust.", "Nachbearbeitung von Scans: Seitentrennung, Begradigung, Beschnitt, Bereinigung, PDF mit OCR. Clean-Room-Implementierung in Rust.", "Szkennelt lapok utómunkája: oldalszétválasztás, kiegyenesítés, vágás, tisztítás, PDF OCR-rel. Clean-room megvalósítás Rustban."]),
    ("btn.add", ["Adaugă", "Add", "Ajouter", "Aggiungi", "Añadir", "Adicionar", "Hinzufügen", "Hozzáadás"]),
    ("btn.remove", ["Elimină", "Remove", "Retirer", "Rimuovi", "Quitar", "Remover", "Entfernen", "Eltávolítás"]),
    ("btn.clear", ["Golește", "Clear", "Vider", "Svuota", "Vaciar", "Limpar", "Leeren", "Törlés"]),
    ("btn.up", ["Sus", "Up", "Monter", "Su", "Subir", "Subir", "Nach oben", "Fel"]),
    ("btn.down", ["Jos", "Down", "Descendre", "Giù", "Bajar", "Descer", "Nach unten", "Le"]),
    ("btn.cancel", ["Anulează", "Cancel", "Annuler", "Annulla", "Cancelar", "Cancelar", "Abbrechen", "Mégse"]),
    ("list.title", ["Pagini", "Pages", "Pages", "Pagine", "Páginas", "Páginas", "Seiten", "Oldalak"]),
    ("list.empty", ["Trage aici imagini sau un folder, ori apasă „Adaugă”.", "Drop images or a folder here, or press “Add”.", "Déposez des images ou un dossier ici, ou appuyez sur « Ajouter ».", "Trascina qui immagini o una cartella, oppure premi «Aggiungi».", "Suelta aquí imágenes o una carpeta, o pulsa «Añadir».", "Largue aqui imagens ou uma pasta, ou prima «Adicionar».", "Bilder oder einen Ordner hier ablegen oder „Hinzufügen“ drücken.", "Húzzon ide képeket vagy mappát, vagy nyomja meg a „Hozzáadás” gombot."]),
    ("drop.hint", ["Eliberează pentru a adăuga", "Release to add", "Relâchez pour ajouter", "Rilascia per aggiungere", "Suelta para añadir", "Largue para adicionar", "Zum Hinzufügen loslassen", "Engedje el a hozzáadáshoz"]),
    ("preview.empty", ["Adaugă scanări pentru a începe", "Add scans to get started", "Ajoutez des numérisations pour commencer", "Aggiungi delle scansioni per iniziare", "Añade escaneos para empezar", "Adicione digitalizações para começar", "Fügen Sie Scans hinzu, um zu beginnen", "Adjon hozzá szkenneléseket a kezdéshez"]),
    ("preview.blank", ["pagină goală", "blank page", "page blanche", "pagina vuota", "página en blanco", "página em branco", "leere Seite", "üres oldal"]),
    ("preview.skew", ["înclinare corectată", "skew corrected", "inclinaison corrigée", "inclinazione corretta", "inclinación corregida", "inclinação corrigida", "Schräglage korrigiert", "dőlés korrigálva"]),
    ("view.both", ["Alăturat", "Side by side", "Côte à côte", "Affiancati", "Lado a lado", "Lado a lado", "Nebeneinander", "Egymás mellett"]),
    ("view.original", ["Original", "Original", "Original", "Originale", "Original", "Original", "Original", "Eredeti"]),
    ("view.processed", ["Procesat", "Processed", "Traité", "Elaborato", "Procesado", "Processado", "Bearbeitet", "Feldolgozott"]),
    ("view.fit", ["Potrivește", "Fit", "Ajuster", "Adatta", "Ajustar", "Ajustar", "Einpassen", "Illesztés"]),
    ("adj.title", ["Această pagină:", "This page:", "Cette page :", "Questa pagina:", "Esta página:", "Esta página:", "Diese Seite:", "Ez az oldal:"]),
    ("adj.rot_left", ["Rotește la stânga 90°", "Rotate left 90°", "Pivoter à gauche de 90°", "Ruota a sinistra di 90°", "Girar 90° a la izquierda", "Rodar 90° à esquerda", "90° nach links drehen", "Forgatás balra 90°-kal"]),
    ("adj.rot_right", ["Rotește la dreapta 90°", "Rotate right 90°", "Pivoter à droite de 90°", "Ruota a destra di 90°", "Girar 90° a la derecha", "Rodar 90° à direita", "90° nach rechts drehen", "Forgatás jobbra 90°-kal"]),
    ("adj.split_default", ["implicit", "default", "par défaut", "predefinito", "predeterminado", "predefinido", "Standard", "alapértelmezett"]),
    ("adj.skew_manual", ["Înclinare manuală", "Manual skew", "Inclinaison manuelle", "Inclinazione manuale", "Inclinación manual", "Inclinação manual", "Manuelle Schräglage", "Kézi dőlés"]),
    ("adj.reset", ["Resetează", "Reset", "Réinitialiser", "Ripristina", "Restablecer", "Repor", "Zurücksetzen", "Visszaállítás"]),
    ("set.title", ["Setări", "Settings", "Réglages", "Impostazioni", "Ajustes", "Definições", "Einstellungen", "Beállítások"]),
    ("set.input", ["Intrare", "Input", "Entrée", "Ingresso", "Entrada", "Entrada", "Eingabe", "Bemenet"]),
    ("set.dpi", ["Rezoluție (dpi)", "Resolution (dpi)", "Résolution (dpi)", "Risoluzione (dpi)", "Resolución (ppp)", "Resolução (dpi)", "Auflösung (dpi)", "Felbontás (dpi)"]),
    ("set.auto_orient", ["Detectează pagina culcată", "Detect sideways pages", "Détecter les pages couchées", "Rileva le pagine di lato", "Detectar páginas de lado", "Detetar páginas de lado", "Gedrehte Seiten erkennen", "Oldalra fordított lapok felismerése"]),
    ("set.split", ["Separare pagini", "Page split", "Séparation des pages", "Divisione pagine", "Separación de páginas", "Separação de páginas", "Seitentrennung", "Oldalak szétválasztása"]),
    ("split.auto", ["Automat", "Automatic", "Automatique", "Automatica", "Automática", "Automática", "Automatisch", "Automatikus"]),
    ("split.middle", ["La mijloc", "At the middle", "Au milieu", "A metà", "Por la mitad", "Ao meio", "In der Mitte", "Középen"]),
    ("split.off", ["Oprită", "Off", "Désactivée", "Disattivata", "Desactivada", "Desativada", "Aus", "Ki"]),
    ("set.geometry", ["Geometrie", "Geometry", "Géométrie", "Geometria", "Geometría", "Geometria", "Geometrie", "Geometria"]),
    ("set.osd", ["Orientare cu OCR (OSD)", "Orientation via OCR (OSD)", "Orientation par OCR (OSD)", "Orientamento tramite OCR (OSD)", "Orientación por OCR (OSD)", "Orientação por OCR (OSD)", "Ausrichtung per OCR (OSD)", "Tájolás OCR-rel (OSD)"]),
    ("set.osd_tip", ["Întoarce paginile cu susul în jos sau pe o parte folosind Tesseract", "Turns upside-down or sideways pages upright using Tesseract", "Redresse les pages à l’envers ou couchées avec Tesseract", "Raddrizza le pagine capovolte o di lato con Tesseract", "Endereza páginas invertidas o de lado con Tesseract", "Endireita páginas invertidas ou de lado com o Tesseract", "Dreht kopfstehende oder liegende Seiten mit Tesseract auf", "A fejjel lefelé vagy oldalra fordított lapokat a Tesseract állítja talpra"]),
    ("set.osd_missing", ["Lipsește osd.traineddata din instalarea Tesseract", "osd.traineddata is missing from the Tesseract install", "osd.traineddata est absent de l’installation de Tesseract", "osd.traineddata manca nell’installazione di Tesseract", "Falta osd.traineddata en la instalación de Tesseract", "Falta o osd.traineddata na instalação do Tesseract", "osd.traineddata fehlt in der Tesseract-Installation", "Az osd.traineddata hiányzik a Tesseract telepítéséből"]),
    ("set.deskew", ["Îndreaptă automat", "Auto-deskew", "Redressement automatique", "Raddrizzamento automatico", "Enderezado automático", "Endireitamento automático", "Automatisch begradigen", "Automatikus kiegyenesítés"]),
    ("set.max_skew", ["Înclinare maximă", "Max skew", "Inclinaison max.", "Inclinazione max", "Inclinación máx.", "Inclinação máx.", "Max. Schräglage", "Max. dőlés"]),
    ("set.margin", ["Margini", "Margins", "Marges", "Margini", "Márgenes", "Margens", "Ränder", "Margók"]),
    ("set.page_size", ["Dimensiune pagină", "Page size", "Format de page", "Formato pagina", "Tamaño de página", "Tamanho da página", "Seitengröße", "Oldalméret"]),
    ("size.free", ["Liberă", "Free", "Libre", "Libero", "Libre", "Livre", "Frei", "Szabad"]),
    ("size.custom", ["Personalizată", "Custom", "Personnalisé", "Personalizzato", "Personalizado", "Personalizado", "Benutzerdefiniert", "Egyéni"]),
    ("set.output", ["Ieșire", "Output", "Sortie", "Uscita", "Salida", "Saída", "Ausgabe", "Kimenet"]),
    ("mode.color", ["Color", "Colour", "Couleur", "Colore", "Color", "Cor", "Farbe", "Színes"]),
    ("mode.gray", ["Gri", "Gray", "Gris", "Grigio", "Gris", "Cinzento", "Graustufen", "Szürke"]),
    ("mode.bw", ["Alb-negru", "Black & white", "Noir et blanc", "Bianco e nero", "Blanco y negro", "Preto e branco", "Schwarzweiß", "Fekete-fehér"]),
    ("set.uniform", ["Aceeași dimensiune pentru toate paginile", "Same size for all pages", "Même taille pour toutes les pages", "Stessa dimensione per tutte le pagine", "Mismo tamaño para todas las páginas", "Mesmo tamanho para todas as páginas", "Gleiche Größe für alle Seiten", "Minden oldal azonos méretű"]),
    ("mode.mixed", ["Mixt", "Mixed", "Mixte", "Misto", "Mixto", "Misto", "Gemischt", "Vegyes"]),
    ("mode.hybrid", ["Hibrid", "Hybrid", "Hybride", "Ibrido", "Híbrido", "Híbrido", "Hybrid", "Hibrid"]),
    ("set.threshold", ["Metodă prag", "Threshold", "Méthode de seuil", "Metodo di soglia", "Método de umbral", "Método de limiar", "Schwellwert-Verfahren", "Küszöbölési módszer"]),
    ("set.picture_sens", ["Sensib. imagini", "Picture sensitivity", "Sensibilité images", "Sensibilità immagini", "Sensibilidad de imágenes", "Sensibilidade das imagens", "Bildempfindlichkeit", "Képérzékenység"]),
    ("set.hybrid_dpi", ["DPI fundal", "Background dpi", "dpi du fond", "dpi dello sfondo", "ppp del fondo", "dpi do fundo", "Hintergrund-dpi", "Háttér dpi"]),
    ("set.normalize", ["Uniformizează iluminarea", "Even out lighting", "Uniformiser l’éclairage", "Uniforma l’illuminazione", "Uniformar la iluminación", "Uniformizar a iluminação", "Beleuchtung angleichen", "Megvilágítás kiegyenlítése"]),
    ("set.dewarp", ["Îndreaptă rândurile curbate", "Straighten curved lines", "Redresser les lignes courbes", "Raddrizza le righe curve", "Enderezar líneas curvas", "Endireitar linhas curvas", "Gekrümmte Zeilen begradigen", "Görbe sorok kiegyenesítése"]),
    ("set.output_dpi", ["DPI ieșire", "Output dpi", "dpi de sortie", "dpi di uscita", "ppp de salida", "dpi de saída", "Ausgabe-dpi", "Kimeneti dpi"]),
    ("set.sauvola", ["Sensibilitate", "Sensitivity", "Sensibilité", "Sensibilità", "Sensibilidad", "Sensibilidade", "Empfindlichkeit", "Érzékenység"]),
    ("set.despeckle", ["Elimină petele mici", "Remove small specks", "Supprimer les petites taches", "Rimuovi i puntini", "Eliminar motas pequeñas", "Remover pontos pequenos", "Kleine Flecken entfernen", "Apró foltok eltávolítása"]),
    ("set.pdf", ["PDF", "PDF", "PDF", "PDF", "PDF", "PDF", "PDF", "PDF"]),
    ("set.quality", ["Calitate JPEG", "JPEG quality", "Qualité JPEG", "Qualità JPEG", "Calidad JPEG", "Qualidade JPEG", "JPEG-Qualität", "JPEG minőség"]),
    ("set.quality_hint", ["0 = fără pierderi. Paginile alb-negru sunt mereu fără pierderi.", "0 = lossless. Black & white pages are always lossless.", "0 = sans perte. Les pages noir et blanc sont toujours sans perte.", "0 = senza perdita. Le pagine in bianco e nero sono sempre senza perdita.", "0 = sin pérdida. Las páginas en blanco y negro siempre son sin pérdida.", "0 = sem perdas. As páginas a preto e branco são sempre sem perdas.", "0 = verlustfrei. Schwarzweißseiten sind immer verlustfrei.", "0 = veszteségmentes. A fekete-fehér oldalak mindig veszteségmentesek."]),
    ("set.skip_blank", ["Omite paginile goale", "Skip blank pages", "Ignorer les pages blanches", "Salta le pagine vuote", "Omitir páginas en blanco", "Ignorar páginas em branco", "Leere Seiten überspringen", "Üres oldalak kihagyása"]),
    ("set.ocr", ["OCR", "OCR", "OCR", "OCR", "OCR", "OCR", "OCR", "OCR"]),
    ("set.ocr_enable", ["Text căutabil (OCR) în PDF", "Searchable text (OCR) in PDF", "Texte consultable (OCR) dans le PDF", "Testo ricercabile (OCR) nel PDF", "Texto buscable (OCR) en el PDF", "Texto pesquisável (OCR) no PDF", "Durchsuchbarer Text (OCR) im PDF", "Kereshető szöveg (OCR) a PDF-ben"]),
    ("set.ocr_missing", ["Tesseract nu a fost găsit. Instalează-l (și pachetul de limbă dorit) pentru OCR.", "Tesseract was not found. Install it (and the language pack you need) to enable OCR.", "Tesseract est introuvable. Installez-le (avec la langue voulue) pour activer l’OCR.", "Tesseract non trovato. Installalo (con la lingua che ti serve) per attivare l’OCR.", "No se encontró Tesseract. Instálalo (con el idioma que necesites) para activar el OCR.", "Não foi encontrado o Tesseract. Instale-o (com o idioma de que precisa) para ativar o OCR.", "Tesseract wurde nicht gefunden. Installieren Sie es (mit dem benötigten Sprachpaket), um OCR zu aktivieren.", "A Tesseract nem található. Telepítse (a szükséges nyelvvel együtt) az OCR bekapcsolásához."]),
    ("set.ocr_langs", ["Limbi recunoscute", "Recognition languages", "Langues de reconnaissance", "Lingue di riconoscimento", "Idiomas de reconocimiento", "Idiomas de reconhecimento", "Erkennungssprachen", "Felismerési nyelvek"]),
    ("set.psm", ["Mod de segmentare", "Segmentation mode", "Mode de segmentation", "Modalità di segmentazione", "Modo de segmentación", "Modo de segmentação", "Segmentierungsmodus", "Szegmentálási mód"]),
    ("status.hint", ["Pregătit. Alege o pagină pentru previzualizare.", "Ready. Select a page to preview.", "Prêt. Choisissez une page à prévisualiser.", "Pronto. Seleziona una pagina per l’anteprima.", "Listo. Selecciona una página para previsualizar.", "Pronto. Selecione uma página para pré-visualizar.", "Bereit. Wählen Sie eine Seite für die Vorschau.", "Kész. Válasszon egy oldalt az előnézethez."]),
    ("status.added", ["fișier(e) adăugate", "file(s) added", "fichier(s) ajouté(s)", "file aggiunti", "archivo(s) añadido(s)", "ficheiro(s) adicionado(s)", "Datei(en) hinzugefügt", "fájl hozzáadva"]),
    ("status.working", ["Se procesează…", "Working…", "Traitement…", "Elaborazione…", "Procesando…", "A processar…", "Verarbeitung…", "Feldolgozás…"]),
    ("status.pdf_saved", ["PDF salvat", "PDF saved", "PDF enregistré", "PDF salvato", "PDF guardado", "PDF guardado", "PDF gespeichert", "PDF mentve"]),
    ("status.images_saved", ["Imagini salvate în", "Images saved to", "Images enregistrées dans", "Immagini salvate in", "Imágenes guardadas en", "Imagens guardadas em", "Bilder gespeichert in", "Képek mentve ide:"]),
    ("status.pages", ["pagini", "pages", "pages", "pagine", "páginas", "páginas", "Seiten", "oldal"]),
    ("status.words", ["cuvinte recunoscute", "words recognised", "mots reconnus", "parole riconosciute", "palabras reconocidas", "palavras reconhecidas", "Wörter erkannt", "szó felismerve"]),
    ("status.blank_skipped", ["pagini goale omise", "blank pages skipped", "pages blanches ignorées", "pagine vuote saltate", "páginas en blanco omitidas", "páginas em branco ignoradas", "leere Seiten übersprungen", "üres oldal kihagyva"]),
    ("status.failed", ["fișiere eșuate (primul)", "file(s) failed (first)", "fichier(s) en échec (premier)", "file non riusciti (il primo)", "archivo(s) con error (el primero)", "ficheiro(s) com falha (o primeiro)", "Datei(en) fehlgeschlagen (erste)", "fájl sikertelen (az első)"]),
    ("status.crashed", ["procesarea s-a oprit neașteptat", "processing stopped unexpectedly", "le traitement s’est arrêté de façon inattendue", "l’elaborazione si è interrotta inaspettatamente", "el procesamiento se detuvo inesperadamente", "o processamento parou inesperadamente", "die Verarbeitung wurde unerwartet beendet", "a feldolgozás váratlanul leállt"]),
    ("status.cancelled", ["Anulat.", "Cancelled.", "Annulé.", "Annullato.", "Cancelado.", "Cancelado.", "Abgebrochen.", "Megszakítva."]),
    ("status.error", ["Eroare", "Error", "Erreur", "Errore", "Error", "Erro", "Fehler", "Hiba"]),
    ("menu.open_project", ["Deschide proiect…", "Open project…", "Ouvrir un projet…", "Apri progetto…", "Abrir proyecto…", "Abrir projeto…", "Projekt öffnen…", "Projekt megnyitása…"]),
    ("menu.save_project", ["Salvează proiectul…", "Save project…", "Enregistrer le projet…", "Salva progetto…", "Guardar proyecto…", "Guardar projeto…", "Projekt speichern…", "Projekt mentése…"]),
    ("status.project_saved", ["Proiect salvat", "Project saved", "Projet enregistré", "Progetto salvato", "Proyecto guardado", "Projeto guardado", "Projekt gespeichert", "Projekt mentve"]),
    ("status.project_loaded", ["Proiect încărcat", "Project loaded", "Projet chargé", "Progetto caricato", "Proyecto cargado", "Projeto carregado", "Projekt geladen", "Projekt betöltve"]),
    ("adj.content", ["Chenarul conținutului (mm)", "Content box (mm)", "Cadre du contenu (mm)", "Riquadro del contenuto (mm)", "Marco del contenido (mm)", "Caixa do conteúdo (mm)", "Inhaltsrahmen (mm)", "Tartalomkeret (mm)"]),
    ("adj.content_hint", ["Mută fiecare latură spre exterior (+) sau interior (−)", "Move each side outwards (+) or inwards (−)", "Déplace chaque côté vers l’extérieur (+) ou l’intérieur (−)", "Sposta ogni lato verso l’esterno (+) o l’interno (−)", "Mueve cada lado hacia fuera (+) o hacia dentro (−)", "Move cada lado para fora (+) ou para dentro (−)", "Verschiebt jede Seite nach außen (+) oder innen (−)", "Minden oldalt kifelé (+) vagy befelé (−) tol"]),
    ("adj.left", ["Stânga", "Left", "Gauche", "Sinistra", "Izquierda", "Esquerda", "Links", "Bal"]),
    ("adj.top", ["Sus", "Top", "Haut", "Alto", "Arriba", "Cima", "Oben", "Felül"]),
    ("adj.right", ["Dreapta", "Right", "Droite", "Destra", "Derecha", "Direita", "Rechts", "Jobb"]),
    ("adj.bottom", ["Jos", "Bottom", "Bas", "Basso", "Abajo", "Baixo", "Unten", "Alul"]),
    ("set.align", ["Poziționare", "Placement", "Placement", "Posizione", "Posición", "Posição", "Platzierung", "Elhelyezés"]),
    ("align.left", ["Stânga", "Left", "Gauche", "Sinistra", "Izquierda", "Esquerda", "Links", "Balra"]),
    ("align.center", ["Centru", "Centre", "Centre", "Centro", "Centro", "Centro", "Mitte", "Középre"]),
    ("align.right", ["Dreapta", "Right", "Droite", "Destra", "Derecha", "Direita", "Rechts", "Jobbra"]),
    ("align.top", ["Sus", "Top", "Haut", "Alto", "Arriba", "Cima", "Oben", "Felül"]),
    ("align.bottom", ["Jos", "Bottom", "Bas", "Basso", "Abajo", "Baixo", "Unten", "Alul"]),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn table_is_complete_and_unique() {
        let mut seen = HashSet::new();
        for (k, v) in TABLE {
            assert!(seen.insert(*k), "duplicate key {k}");
            for (i, s) in v.iter().enumerate() {
                assert!(!s.is_empty(), "empty text for {k} in {}", Lang::ALL[i].1);
            }
        }
    }

    #[test]
    fn placeholders_and_symbols_survive_translation() {
        // Units and symbols that must appear unchanged in every language.
        for (k, v) in TABLE {
            if *k == "set.quality_hint" {
                assert!(v.iter().all(|s| s.starts_with("0 =")), "{k}");
            }
            if *k == "adj.content_hint" {
                assert!(v.iter().all(|s| s.contains("(+)") && s.contains("(−)")), "{k}");
            }
        }
    }

    #[test]
    fn every_key_used_in_the_app_exists() {
        const GROUPS: &[&str] = &["about", "adj", "align", "btn", "drop", "list", "menu", "mode", "preview", "set", "size", "split", "status", "view"];
        let src = include_str!("app.rs");
        let mut checked = 0;
        for lit in src.split('"').skip(1).step_by(2) {
            let Some((g, rest)) = lit.split_once('.') else { continue };
            if GROUPS.contains(&g) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                assert!(TABLE.iter().any(|(k, _)| *k == lit), "app.rs uses unknown i18n key {lit}");
                checked += 1;
            }
        }
        assert!(checked > 50, "only {checked} keys found — scanner broken?");
    }

    #[test]
    fn lookup_and_fallback() {
        assert_eq!(t(Lang::Ro, "menu.file"), "Fișier");
        assert_eq!(t(Lang::En, "menu.file"), "File");
        assert_eq!(t(Lang::Hu, "menu.file"), "Fájl");
        assert_eq!(t(Lang::En, "no.such.key"), "no.such.key");
    }

    #[test]
    fn locale_codes() {
        assert_eq!(Lang::from_code("pt-BR"), Some(Lang::Pt));
        assert_eq!(Lang::from_code("de_AT.UTF-8"), Some(Lang::De));
        assert_eq!(Lang::from_code("HU"), Some(Lang::Hu));
        assert_eq!(Lang::from_code("ja-JP"), None);
        assert_eq!(Lang::from_code(""), None);
        for (l, code, _) in Lang::ALL {
            assert_eq!(Lang::from_code(code), Some(l));
            assert_eq!(l.code(), code);
        }
    }
}
