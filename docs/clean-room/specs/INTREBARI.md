# Întrebări rămase fără răspuns în specificații (sesiunea de implementare)

Fiecare punct: spec, ce lipsește, ce s-a făcut în cod până la răspuns.

## 01 bagit-validare
1. Secvențe procentuale în afara `%0A`, `%0D`, `%25` ([necunoscut], punctul 6): codul refuză linia (constatare „malformed"), nu ghicește decodarea.
2. Semantica „complet" cu preluări (`fetch.txt`) nefinalizate ([necunoscut], punctul 11): codul nu descarcă nimic și raportează fișierul listat dar absent ca „missing" (pachet incomplet); varianta opusă nu a fost aleasă.
3. Algoritm de manifest necunoscut (altul decât md5, sha1, sha256, sha512): spec-ul nu spune ce se întâmplă; codul raportează eroare „checksum" (nimic nu a fost verificat), deci pachetul nu e valid.
4. Spec-ul nu spune dacă un fișier din `data/` nelistat se raportează o dată per manifest sau o dată în total; codul îl raportează per manifest.

## 02 merkle-dovezi
1. Prefixele de frunză/nod (0x00 / 0x01) erau [necunoscut] în spec; au fost confirmate din textul public RFC 9162, 2.1.1 (citit la implementare) și coincid cu implementarea existentă.
2. Codificarea frunzei rămâne cea existentă (nume + hash de fișier), fixată de proiect; spec-ul o lasă utilizatorului. Ordinea de inserare este opțională (`--merkle-keep-order`), deoarece comportamentul existent sortează după nume și rădăcinile deja publicate nu trebuie schimbate.

## 03 mrz-icao9303
1. Transliterarea literelor non-A-Z (Partea 3, sec. 6) și lista codurilor de stat: [necunoscut]; `encode_name` refuză astfel de caractere, codurile de stat nu se validează.
2. Secolul pentru AA: [necunoscut]; nu se deduce, datele rămân AALLZZ.
3. TD2 (2×36): spec-ul zice că programul îl ignoră; codul existent îl citea deja, a rămas neschimbat ca funcționalitate (fără reguli noi), nespecificat.
4. Metoda de scurtare a numelui rămâne la discreția statului; politica aleasă (primarul tăiat, secundarul tăiat la locul rămas) este a implementării. Dacă tăierea cade pe `<`, ultima poziție poate rămâne `<` (nesemnalat ca trunchiat).
5. Reparația: tabelul existent (O Q D→0, I L→1, Z→2, S→5, B→8) este mai larg decât cele cinci confuzii din spec (O, I, S, B, Z); neschimbat pentru a nu regresa comportamentul livrat.

## 04 en16931-reguli-factura
1. Căile XML UBL ale termenilor BT: [necunoscut] în spec. Pagina Peppol cu sintaxa UBL a putut fi citită doar pentru `cac:LegalMonetaryTotal` (nume de elemente); celelalte pagini nu au putut fi preluate (în afara listei permise). Cititorul `from_ubl` citește doar: BT-1, 2, 5, 24, 44, 31, 48, 106, 107, 108, 109, 110, 112, 113, 114, 115 și, pe linie, BT-126 și BT-131. Nu sunt citite din XML (regulile există și sunt testate pe modelul semantic, dar nu rulează pe XML): tip document BT-3, țări BT-40/55, cantitate/unitate/nume/preț pe linie, reduceri/taxe (document și linie), defalcarea TVA (BT-116..119), perioade, punct fiscal, reprezentant fiscal.
2. Mod de rotunjire la BR-CO-17: neprecizat; se acceptă orice valoare la cel mult 0,005 de produsul exact. Toleranță la celelalte comparații: neprecizată; comparație exactă (zecimale exacte, fără virgulă mobilă).
3. Semnul la notă de credit: neprecizat; aceleași reguli, fără interpretare de semn.
4. Corespondența BR-DEC-nn ↔ câmp: [necunoscut]; încălcarea se raportează ca `BR-DEC` cu câmpul BT. Zecimale numărate după valoare (zerourile finale nu contează).
5. Reguli neimplementate: BR-CO-22/24 (taxe: motiv), BR-33/36/38/42/43/44 (corespondența exactă număr ↔ sumă/motiv taxă sau reducere nu e în spec; sunt implementate doar BR-31, BR-41, BR-CO-21, BR-CO-23 din testul 8), BR-CO-05..08 (liste de coduri), BR-49, BR-56, BR-65, regulile condiționale 19, 20, 48, 50, 53, 55, 61 (necitite), BR-CL-* (liste de coduri necitite).
6. BR-CO-09: verificat doar că primele două caractere sunt litere; lista codurilor de țară nu a fost citită.

## 05 epub3-structura-accesibilitate
1. Poziția și compresia lui `mimetype`, regulile OCF, `dcterms:modified`, regulile de manifest/spină 5.6-5.7 și cerințele navigării: [necunoscut]; nu sunt verificate (doar prezența lui `mimetype`).
2. Valorile admise pentru accessibilityHazard / accessMode / accessibilityFeature (vocabular Schema.org): necitite. Cartea scrisă de `--epub` nu are `accessibilityHazard`, deci `--verify-epub` o raportează cu o eroare; scriitorul nu a fost modificat, pentru că valoarea corectă nu e în spec. Nu s-a scris nici accessibilitySummary.
3. Cum se identifică documentul de navigare (proprietatea `nav` în manifest) și marcajele de pagină (`epub:type="pagebreak"`) nu e în spec; s-a preluat din codul existent al scriitorului EPUB.
4. Numele proprietății care declară sursa paginării: nespecificat; verificarea acceptă orice `dc:source` nevid ca declarație. Forma exactă a declarației `dcterms:conformsTo` (meta cu property) și a certificatorului (`a11y:certifiedBy`) este luată din textul spec-ului.
5. Șirurile exacte de tipuri media pentru fiecare tip de bază nu sunt în spec (doar numele formatelor); s-au folosit numele IANA obișnuite. „Alternativă intrinsecă" (fără lanț în manifest) nu e implementată: o resursă străină fără `fallback` e eroare.
6. Existența în container a fiecărui element din manifest este verificată (consecința „restul stau în container"); nu e regulă explicită în spec.

## 06 tei-facsimil
1. Numele elementelor și atributelor TEI nu sunt toate în spec (acolo: ulx, uly, lrx, lry și descrieri). Folosite: `facsimile`, `surface`, `zone`, `graphic` (cu atributul `url` pentru fișier), atributul de legătură `facs` (pagina att.global.facs citită confirmă existența clasei și `facsimile`/`graphic`), atributul de început al suprafeței `start`, `xml:id`. Numele `surface`, `zone`, `url`, `start` nu au putut fi confirmate din textul preluat; de reconfirmat cu documentul oficial.
2. Axa y (origine sus-stânga): [necunoscut] în spec, aleasă ca decizie a implementării (pixelii imaginii scrise, origine sus-stânga).
3. Validarea față de schema TEI nu s-a făcut; testele verifică bună-formare, unicitatea identificatorilor și rezolvarea legăturilor.
4. Imaginea legată este pagina prelucrată (după îndreptare/decupare), scrisă ca PNG de program în DIR; nu originalul. Atribute de dimensiune pe `graphic` nu s-au scris (nu sunt în spec).
5. Zone doar pentru paragrafe, titluri și tabele; figuri nu există în modelul de blocuri, deci nu au zone. Rectangle-ul unui bloc este încadrarea cuvintelor lui (reuniunea liniilor).

## 07 dnssd-descoperire
1. Transportul (mDNS, RFC 6762) nu a fost citit: nicio interogare nu se trimite; nu există opțiune CLI și nu s-a legat de `--scan-escl`. Tipul de serviciu al scannerelor și cheile TXT: necunoscute. Implementat doar modulul logic `dnssd` (nume, TXT, SRV/PTR/adrese -> listă), cu teste; **parțial**.
2. Normalizarea NFC a numelui de instanță: nu se aplică (ar cere tabele Unicode); doar lungime (63 de octeți) și caractere de control.
3. SRV cu port 0: spec-ul zice și „port 0, pondere 0, prioritate 0 = substituent", și testul 6 „port 0 → nu se afișează"; codul ascunde orice port 0. Alegerea între SRV-uri: prioritate minimă, apoi pondere maximă (deterministă, nu aleatoare cum cere ponderarea clasică).
4. Cheile TXT invalide (goale sau cu caractere neimprimabile) sunt omise; „ghilimele" în valori nu sunt tratate.
5. Duplicatele pe interfețe: instanța e comparată după nume fără diferență de majuscule ASCII, adresele unite (regula pe interfețe: [necunoscut]).
6. Compatibilitatea cu modul offline/sigilat: decizia omului ([necunoscut]); modulul nu face rețea.

## 08 fadgi-niveluri
1. Rândurile OECF și eroare de balans de alb („+9 ≤ 8" etc.): sensul notației [necunoscut]; **nu sunt evaluate** (raportate „not evaluated"). Paranteza de la zgomot: [necunoscut]; zgomotul se evaluează doar cu prima limită din fiecare pereche, strict „<" (decizia din testul 6).
2. Regula de nivel global: [necunoscut]; nu se calculează. Se listează doar rândurile limitative (cele cu cel mai mic număr de stele dintre cele evaluate).
3. 300 ppi se raportează 3 stele (cel mai înalt nivel care îl satisface; valoarea e aceeași pentru 2 și 3). „±N" la scara de reproducere tratat ca limită inclusivă.
4. „Jumătate din frecvența de eșantionare" = 0,5 cicluri/pixel (deducție); cu `--measure-edge`, MTF50 % = mtf50/0,5×100 și SFR la 0,5 citit prin interpolare din curbă. Eficiența de eșantionare MTF10 nu se calculează din margine (definiția nu e în spec): se dă manual.
5. Ediția 2023/2024 nu a fost citită; alte categorii de materiale nu sunt suportate (refuz).

## 09 pades-lt-lta
1. Obținerea valorilor de revocare (OCSP, CRL) și validarea lanțului/revocării: [necunoscut] (RFC 5280 / RFC 6960 necitite). Nimic nu se preia din rețea; valorile se dau de utilizator (`--dss-cert/--dss-ocsp/--dss-crl`) și sunt doar verificate să fie un singur element DER. Nivelul „B-LT" raportat înseamnă „depozitul conține certificate și cel puțin o valoare de revocare", nu „validat". Verificarea ca depozitul să acopere *fiecare* semnătură (certificatele fiecărui semnatar) nu se face.
2. Mărimea spațiului rezervat: nu e dată de spec; rămâne cea existentă (20000 octeți) și un token care nu încape face operația să eșueze înainte de scriere. Nu s-a verificat că 20000 ajunge pentru token-uri reale ale autorităților publice.
3. Dicționarul VRI nu se scrie (spec: opțional, „nu ar trebui folosit"); depozitul existent se păstrează, inclusiv un VRI deja prezent. Fișierele cu flux de referințe încrucișate (xref stream) sunt refuzate de `--pades-lt`/`--pades-lta` (semnarea le rescrie clasic, aceste operații nu).
4. Reînnoirea ștampilei (reguli): [necunoscut]. `--pades-lta` doar adaugă o ștampilă nouă peste un depozit existent; adăugarea materialului nou pentru ștampila anterioară e responsabilitatea utilizatorului (`--pades-lt` apoi `--pades-lta`).
5. Imprint: se compară doar rezumat SHA-256; alte algoritmi sunt raportați ca „necomparat". Semnătura token-ului ștampilei nu e verificată de `--pades-level`.
6. Nivelul B-T: un atribut de ștampilă pe semnătură este recunoscut după OID-ul lui; la B-LTA se cere o ștampilă de document care acoperă depozitul, depozitul fiind după semnătură.
7. **Constatare:** semnătorul existent (`openssl cms -sign -cades`, `--sign-pdf`) produce un atribut CMS de timp de semnare, pe care spec-ul îl interzice; `--pades-level` îl semnalează ca eroare (verificat pe OpenSSL 3.0.13). Nu s-a modificat semnătorul (în afara specului); decizia: titularul.
8. Valoarea V și celelalte verificări „ar trebui" ale ștampilei sunt avertismente, nu erori, conform spec-ului. Regula DocMDP nu e verificată.
