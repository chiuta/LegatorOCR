# Index specificații candidate, lotul 1 (2026-11-09)

Toate specificațiile sunt scrise exclusiv din documentație publică (vezi JURNAL.md), fără cod, fără analiză de binare. Statusul „candidat" înseamnă că nu au trecut de filtrul uman (căsuța „Aprobată de mine" este nebifată peste tot) și că bifa „Specificația nu depinde de brevete" este nebifată peste tot (declarațiile IPR ale surselor nu au fost verificate). Textele surselor au fost citite printr-un instrument de preluare care rezumă pagina, deci fiecare spec are o secțiune „Necunoscute" care trebuie parcursă de om cu documentul oficial în față.

| Nr. | Spec | Status | Sursă publică | Dim. | Depinde de | Ordine recomandată |
|---|---|---|---|---|---|---|
| 01 | [bagit-validare](01-bagit-validare.md) | candidat | IETF RFC 8493 | S | funcția existentă de pachet BagIt | 1 |
| 02 | [merkle-dovezi](02-merkle-dovezi.md) | candidat | IETF RFC 9162 (2.1.1, 2.1.3, 2.1.4) | M | registrul Merkle existent | 2 |
| 03 | [mrz-icao9303](03-mrz-icao9303.md) | candidat | ICAO Doc 9303, Părțile 3, 4, 5 | M | OCR; verificatorul de documente existent | 3 |
| 04 | [en16931-reguli-factura](04-en16931-reguli-factura.md) | candidat (surse parțiale) | lista de reguli EN 16931-UBL din documentația Peppol | M | citirea e-Factura existentă | 4 |
| 05 | [epub3-structura-accesibilitate](05-epub3-structura-accesibilitate.md) | candidat (EPUB 3.3 citit parțial) | W3C EPUB 3.3, EPUB Accessibility 1.1 | M | ieșirea EPUB existentă | 5 |
| 06 | [tei-facsimil](06-tei-facsimil.md) | candidat | TEI P5, cap. 11 (ediția 1.3.0 arhivată) | M | ieșirea TEI existentă | 6 |
| 07 | [dnssd-descoperire](07-dnssd-descoperire.md) | candidat (doar RFC 6763) | IETF RFC 6763 | S | achiziția din scanner (parțială) | 7 |
| 08 | [fadgi-niveluri](08-fadgi-niveluri.md) | candidat (ediția 2016, un singur tabel) | FADGI Technical Guidelines, 2016 | M | măsurătorile de claritate existente | 8 |
| 09 | [pades-lt-lta](09-pades-lt-lta.md) | candidat | ETSI EN 319 142-1 | L | semnarea PDF existentă (nivel B-T) | 9 |
| 10 | [pdfua1-autoverificare](10-pdfua1-autoverificare.md) | candidat, cu rezervă majoră (protocolul oficial necitit) | PDF Association, Matterhorn Protocol 1.1 (rezumate secundare) | L | PDF etichetat existent | 10 |

Ordinea recomandată: întâi cele cu sursă primară citită aproape integral și testabile din standard (01, 02, 03), apoi cele cu goluri moderate (04-06), apoi cele care depind de documente încă necitite sau de decizii umane (07-10). 10 nu ar trebui trimisă la implementare înainte ca omul să obțină documentul Matterhorn oficial.
