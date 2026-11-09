# Funcție: Conformitate structurală și metadate de accesibilitate pentru EPUB 3

Sursă publică: W3C, „EPUB 3.3" (Recommendation), https://www.w3.org/TR/epub-33/ (citit parțial: sumar, secțiunile 1.2, 2, 3.2-3.9, 4.1; rezumat de instrument); W3C, „EPUB Accessibility 1.1", https://www.w3.org/TR/epub-a11y-11/ (primele ~100000 de caractere din 102917: secțiunile 2.2, 2.3, 3.4.1, 3.5.2, 3.5.3); W3C, „EPUB 3 Overview" (Note), https://www.w3.org/TR/2022/NOTE-epub-overview-33-20220517/ . Citite 2026-11-09.
Stare: candidat. Dimensiune: M.

## Scop
Cartea EPUB produsă din textul OCR trebuie să respecte structura cerută de standard și să poarte metadatele de accesibilitate pe care Accesibilitatea EPUB 1.1 le cere, ca să poată fi verificată înainte de predare (în prezent structura este citită înapoi, dar nu validată).

## Intrări / ieșiri
- Intrare: fișier .epub produs (sau unul extern).
- Ieșire: listă de constatări (eroare, avertisment) cu locul în arhivă și cerința încălcată; verdict pentru conformitatea declarată.

## Comportament observabil
**Container.** Un singur fișier ZIP (OCF) cu fișierul „mimetype" și directorul „META-INF"; „container.xml" din acesta indică documentul de pachet [observat, 1.2, 4.1]. Poziția și compresia lui „mimetype" (regula 4.3.2): [necunoscut] (nu a fost în fragmentul citit).
**Pachet.** Fiecare publicație are un document de pachet cu: metadate obligatorii dc:identifier, dc:title, dc:language (5.5.3.1) [observat doar titlul secțiunilor]; atributul unique-identifier; manifest; spină (cel puțin una) [observat]. Regulile de manifest/spină din 5.6-5.7: [necunoscut]. Prefixul dc: trebuie declarat [observat, 1.6]. Obligativitatea dcterms:modified: [necunoscut] (nu a fost în fragmentul citit).
**Document de navigare.** Fiecare publicație include un document de navigare XHTML [observat, 2]; tipurile de nav sunt toc, page-list, landmarks (7.4.2-7.4.4) [observat doar titlurile]; cerințele lor de detaliu: [necunoscut].
**Documente de conținut.** XHTML cu sintaxa XML (profil HTML, 6.1) sau SVG; resursele bazate pe XML sunt XML 1.0 bine format, fără entități externe în DTD intern, fără XInclude, codificate UTF-8 sau UTF-16 (UTF-8 recomandat) [observat, 3.9].
**Tipuri media.** Tipurile de bază permise fără alternativă includ GIF, JPEG, PNG, SVG, WebP, MP3, AAC, Opus, CSS, TrueType, OpenType, WOFF, WOFF2, XHTML, JavaScript, NCX, SMIL; orice altceva este resursă străină și cere alternativă (lanț de rezervă în manifest sau alternativă intrinsecă); lanțurile nu se autoreferă și nu fac cicluri; o resursă străină din spină are lanț care ajunge la un document de conținut EPUB [observat, 3.2-3.5].
**Locația resurselor.** Audio, video, fonturi și resurse aduse prin API-uri de script pot fi la distanță; restul stau în container; adresele la distanță ar trebui să fie https; adresele data: nu sunt permise în href-urile documentului de pachet; adresele file: nu sunt permise [observat, 3.6-3.8].
**Metadate de descoperire a accesibilității** [observat, A11y 1.1, 2.2-2.3]: obligatorii accessMode, accessibilityFeature, accessibilityHazard; recomandate accessibilitySummary și accessModeSufficient; metadatele aflate doar într-o înregistrare legată nu satisfac cerința. Exemple de valori din text: accessMode textual, visual, auditory, tactile; feature alternative text, extended descriptions, captions; hazard flashing, motion simulation, sound; lista completă este în vocabularul Schema.org, necitit: [necunoscut].
**Declarație de conformitate** [observat, 3.5.2]: dcterms:conformsTo este obligatoriu, cu valoarea exactă în forma „EPUB Accessibility <versiune> - WCAG <versiune> Level <A|AA|AAA>", versiunea de accesibilitate cel puțin 1.1, WCAG cel puțin 2.0; valoarea „none" poate indica o excepție. Dacă se declară conformitate, certificatorul (a11y:certifiedBy, 3.5.3.1) este obligatoriu; data evaluării, acreditarea și raportul se includ când sunt disponibile [observat]. Programul nu certifică: dacă nimeni nu a evaluat cartea, el nu scrie conformsTo, ci scrie doar metadatele de descoperire pe care le poate sprijini [dedus din 3.5].
**Navigare pe pagini** [observat, 3.4.1]: dacă publicația are echivalent tipărit, lista de pagini (page-list) este obligatorie și leagă toate marcajele de pagină; sursa paginării (ediția tipărită) se declară în metadate; o publicație digitală fără echivalent static nu își numește singură ediția ca sursă. Legătură cu opțiunea de etichete de pagină: etichetele pot alimenta lista [dedus].

## Cazuri limită și erori
- Fără text OCR, cartea e doar imagini: accessMode nu poate afirma „textual" [dedus].
- Resurse străine fără alternativă → eroare. Ciclu în lanțul de alternative → eroare.
- Valoare conformsTo cu versiune sub minim sau format diferit → eroare.
- Marcaje de pagină fără sursă declarată → avertisment.

## Teste de acceptanță
1. Arhivă fără „META-INF" sau fără „container.xml" → eroare de container.
2. Pachet fără dc:title sau dc:language sau dc:identifier → eroare de metadate.
3. Publicație fără document de navigare → eroare.
4. Document de conținut cu XML care nu este bine format sau cu entitate externă → eroare.
5. Manifest cu o imagine în format TIFF (fără alternativă) → eroare de resursă străină; același TIFF cu lanț de alternative către PNG → fără eroare.
6. Metadate cu accessMode și accessibilityFeature dar fără accessibilityHazard → eroare; fără accessibilitySummary → avertisment.
7. dcterms:conformsTo = „EPUB Accessibility 1.1 - WCAG 2.2 Level AA" → format acceptat; „EPUB Accessibility 1.0 - WCAG 2.2 Level AA" → respins; „none" → acceptat ca excepție.
8. Pagini marcate cu page-list dar fără sursă de paginare declarată → avertisment; cu sursă tipărită declarată → fără avertisment.

## Necunoscute
- Fragmentele citite s-au oprit la 4.1 în EPUB 3.3: regulile pachetului, ale navigării, ale regulilor OCF, `mimetype`, dcterms:modified, accesibilitate pentru conținut (WCAG) sunt necitite.
- Fragmentul A11y 1.1 s-a oprit aproape de final; secțiunile de după 3.5.3 sunt necitite.
- Un al doilea control, independent de implementare (validator extern), rămâne decizia omului; nu este numit aici.

## Checklist de filtru
Spec: 05-epub3-structura-accesibilitate   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (neverificat)
- [ ] Aprobată de mine înainte de predare
