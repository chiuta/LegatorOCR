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
