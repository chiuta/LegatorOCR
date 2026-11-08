# Sesiune de ANALIZĂ (REA activ)

Rol: studiezi comportamentul unei ținte și produci DOAR specificații funcționale.

## Reguli stricte
- Nu scrie cod de produs în acest folder.
- Specificațiile NU conțin: cod, pseudocod, asamblare, nume interne de funcții/variabile/clase, texte de UI, asset-uri, branding.
- Descrie comportamentul observabil: intrări → ieșiri, formate de fișiere, protocoale, cazuri limită, erori.
- Marchează fiecare afirmație: [observat] / [dedus] / [necunoscut].
- Preferă observarea comportamentului (rulare, capturi de rețea, formate de fișiere) în loc de decompilare, ori de câte ori e suficient.
- Output-ul brut REA rămâne în `evidence/` și nu iese din acest folder.
- Înainte de orice analiză, completează în `jurnal.md`: data, ținta, versiunea, licența/EULA și dacă permite analiza.
- Specificațiile candidate se scriu după `../2-filtru/spec-template.md` și se depun în `../2-filtru/candidat/`.

## Țintă pentru LegatorOCR
- Țintă: ScanTailor (post-procesare scanări) și, pentru funcții recomandate, ABBYY FineReader / Agent Ransack
- Licență/risc: ScanTailor este GPL: NU copia cod și nu porta fragmente, altfel proiectul MIT devine derivat GPL. ABBYY FineReader și Agent Ransack sunt proprietare: verifică EULA.
