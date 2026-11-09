# Funcție: Verificare completă a unui pachet BagIt 1.0

Sursă publică: IETF RFC 8493, „The BagIt File Packaging Format (V1.0)", J. Kunze et al., octombrie 2018, https://datatracker.ietf.org/doc/html/rfc8493 (citit 2026-11-09 prin instrument de preluare cu rezumare; secțiunile 1.3, 2, 2.1-2.4, 3, 5, 6.1).
Stare: candidat. Dimensiune: S.

## Scop
Utilizatorul primește sau produce un director-pachet de arhivare și vrea un verdict clar: pachetul este „complet" (toate elementele cerute există) și/sau „valid" (toate sumele de control corespund), cu lista exactă a problemelor. Extinde verificarea de bază existentă cu regulile pe care RFC-ul le cere explicit.

## Intrări / ieșiri
- Intrare: calea unui director de bază (numele lui este liber [observat, 2]).
- Ieșire: (a) verdict „complet" da/nu; (b) verdict „valid" da/nu (valid implică complet [observat, 1.3]); (c) listă de constatări, fiecare cu categorie, cale relativă și explicație; (d) cod de ieșire diferit de zero când pachetul nu este valid.
- Categorii de constatări: structură (element cerut lipsă), fișier listat dar absent, fișier de încărcătură util (payload) nelistat, sumă de control nepotrivită, cale nesigură, linie malformată, codificare, avertisment de compatibilitate.

## Comportament observabil
1. Directorul de bază trebuie să conțină fișierul de declarare, subdirectorul de date numit „data" și cel puțin un manifest de încărcătură [observat, 2, 2.1].
2. Declararea conține exact două linii, în ordine: versiunea (forma „M.N", aici 1.0) și codificarea fișierelor-etichetă; declarația este UTF-8 și nu are marcaj de ordine a octeților [observat, 2.1.1]. Codificarea recomandată este UTF-8; alta e permisă dacă e înregistrată IANA [observat].
3. Fiecare manifest de încărcătură are numele „manifest-<algoritm>" cu extensia .txt; numele algoritmului se normalizează scriind cu litere mici și eliminând caracterele nealfanumerice (SHA-256 devine sha256) [observat, 2.4].
4. Fiecare manifest listează fiecare fișier din „data" exact o dată [observat, 2.1.3]. Un fișier din „data" care lipsește din oricare manifest face pachetul incomplet [observat, 3 punctul 4].
5. Fiecare linie de manifest: sumă hexazecimală (litere mari sau mici acceptate), unul sau mai multe spații ori tabulatoare, calea relativă cu „/" ca separator [observat, 2.1.3]. Nu există limită de lungime a căii.
6. Escapare: în căi, doar caracterele LF, CR, CRLF și „%" sunt codificate procentual (RFC 3986); verificatorul decodifică acele secvențe înainte de a deschide fișierul [observat, 2.1.3]. Alte secvențe procentuale: tratamentul este [necunoscut].
7. Manifestele nu pot referi fișiere din afara „data" sau directoare [observat, 2.1.3]. Orice cale din manifest, din manifestul de etichete sau din lista de preluare care ar duce în afara pachetului („/" inițial, „..", „~utilizator") este refuzată fără a citi fișierul [observat, 5.1].
8. Manifestele de etichete (nume „tagmanifest-<algoritm>"): același format de linie; nu listează fișiere din „data" (nicio cale nu începe cu „data/"); listează toate manifestele de încărcătură, nu listează niciun manifest de etichete; ar trebui să listeze și celelalte fișiere-etichetă prezente; ar trebui să folosească aceiași algoritmi ca manifestele de încărcătură [observat, 2.2.1].
9. Fișierul de metadate (bag-info.txt), dacă există: linii „etichetă: valoare", etichetă fără două puncte și fără spații la capete, continuare pe linii indentate, ordinea păstrată; etichetele rezervate sunt insensibile la majuscule [observat, 2.2.2]. „Bagging-Date" are forma AAAA-LL-ZZ; „Payload-Oxum" are forma „octeți.număr-fișiere", apare cel mult o dată [observat].
10. Verificare rapidă: dacă „Payload-Oxum" există și nu coincide cu numărul real de octeți și fișiere din „data", pachetul este raportat incomplet fără a calcula sume; oxum nu înlocuiește verificarea sumelor [observat, 2.2.2].
11. Lista de preluare (fetch.txt), dacă există: linii „adresă lungime cale"; adresa este URI absolut cu spații/tab codificate; lungimea este un număr de octeți sau „-"; fiecare cale listată apare în fiecare manifest de încărcătură; nu listează fișiere-etichetă [observat, 2.2.3]. Programul nu descarcă nimic implicit; ce înseamnă „complet" când fișierele din listă lipsesc încă este [necunoscut].
12. Pachetul este „valid" dacă este complet și fiecare sumă din fiecare manifest (de încărcătură și de etichete) corespunde fișierului [observat, 3].
13. Algoritmi: SHA-256 și SHA-512 sunt obligatorii în unelte; la creare se recomandă SHA-512 implicit; MD5 și SHA-1 se acceptă pentru compatibilitate [observat, 2.4].

## Formate / protocoale
Fișierele sunt text pe linii; terminator LF, CR sau CRLF; numele fișierelor-etichetă text se termină în .txt [observat, 2.3]. Marcajul de ordine a octeților este permis doar unde codificarea îl cere, deci niciodată cu UTF-8 [observat].

## Cazuri limită și erori
- Manifest vechi cu asterisc în fața căii sau alte sfârșituri de linie: acceptat doar cu avertisment că pachetul nu trece verificarea strictă [observat, 6.1.3]. Oferirea rescrierii manifestului este recomandată [observat].
- Director gol în încărcătură: nu poate fi reprezentat; practica este un fișier-marcator, ex. „.keep" [observat, 2.1.3].
- Lungimea din lista de preluare: la preluare se abandonează când se depășește; nu se folosește la alocări critice [observat, 5.3].
- Răspunsul exact la erori nu este impus de RFC (jurnal liber) [observat, 6.1]; formatul raportului este alegerea implementării.
- Semnături digitale și rezistența la atac activ sunt în afara RFC [observat, 5.4].

## Teste de acceptanță
1. Pachet minim (declarație cu două linii, un fișier în „data", un manifest sha256 corect) → complet: da, valid: da.
2. Același pachet plus un fișier în „data" nelistat în manifest → complet: nu (constatare „nelistat"); valid: nu.
3. Un octet modificat într-un fișier din „data" → complet: da, valid: nu (constatare „sumă nepotrivită").
4. Două fișiere de 10 și 5 octeți → „Payload-Oxum" corect este 15.2; valoare declarată 15.3 → incomplet fără calcul de sume.
5. Un fișier numit cu „%" în nume apare în manifest cu „%25"; verificatorul găsește fișierul; o cale „../x" sau „/etc/x" în manifest → refuzată cu eroare de cale nesigură.
6. Declarație cu marcaj de ordine a octeților, sau cu liniile în ordine inversă, sau cu trei linii → constatare de structură.
7. Manifest de etichete care conține „data/a.txt" → constatare; manifest de etichete care nu listează un manifest de încărcătură → constatare.
8. Sumă hexazecimală cu litere mari → acceptată; aceeași cale listată de două ori în același manifest → constatare.

## Necunoscute
- Tratamentul secvențelor procentuale în afara celor patru caractere specificate.
- Semantica „complet" cu preluări nefinalizate.
- Reguli ale bag-info dincolo de etichetele rezervate (nu au fost citite exemplele Anexei).
- Textul citit este un rezumat produs de instrumentul de preluare; formulările exacte „trebuie/ar trebui" ale secțiunilor 2.2.1 și 6.1.3 trebuie reconfirmate de omul care aprobă.

## Checklist de filtru
Spec: 01-bagit-validare   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`, la cererea sesiunii)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (nu am verificat declarațiile IPR ale sursei)
- [ ] Aprobată de mine înainte de predare
