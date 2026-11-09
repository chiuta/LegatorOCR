# Funcție: Arbore de integritate Merkle cu dovezi de includere și de consistență

Sursă publică: IETF RFC 9162, „Certificate Transparency Version 2.0", decembrie 2021, https://www.rfc-editor.org/rfc/rfc9162.html (citit 2026-11-09 prin instrument de preluare cu rezumare; secțiunile 2.1.1, 2.1.3, 2.1.4). RFC 6962 este înlocuit de RFC 9162 [observat în ROADMAP-ul proiectului; comparația nu a fost făcută].
Stare: candidat. Dimensiune: M.

## Scop
Pentru o colecție de fișiere cu sume de control, utilizatorul vrea o „rădăcină" unică publicabilă, apoi (a) să dovedească rapid că un fișier face parte din colecție și (b) să dovedească, după ce colecția a crescut, că noua stare conține starea veche neschimbată (adăugare fără rescriere). Aceasta specifică comportamentul conform RFC 9162 pentru verificare și generare a dovezilor, inclusiv a celor de consistență, care lipsesc din varianta actuală.

## Intrări / ieșiri
- Intrare: listă ordonată de frunze (conținutul fiecărei frunze, de ex. linia de sumă de control a unui fișier); un indice de frunză; mărimi de arbore m < n; rădăcini așteptate.
- Ieșiri: rădăcina arborelui; dovada de includere (listă ordonată de hash-uri); dovada de consistență (listă ordonată de hash-uri); verdict da/nu la verificare, cu motivul eșecului.

## Comportament observabil
1. O singură funcție de hash, aleasă la crearea registrului și neschimbată ulterior; toate hash-urile de nod au lungimea ei [observat, 2.1.1/4.1]. Alegerea SHA-256 este [dedus] din practica proiectului, nu din RFC (RFC-ul acceptă orice algoritm din registrul IANA).
2. Arbore gol: rădăcina este hash-ul șirului vid, fără prefix [observat, 2.1.1]. Valoarea SHA-256 a șirului vid este e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 [dedus din definiția algoritmului].
3. O frunză: hash-ul conținutului cu un octet-prefix de frunză adăugat în față [observat]. Valoarea prefixului nu a fost extrasă din sursa citită: [necunoscut]; implementarea o preia din secțiunea 2.1.1.
4. n > 1 frunze: se ia cea mai mare putere a lui doi strict mai mică decât n, notată k; rădăcina este hash-ul, cu octet-prefix de nod, al concatenării rădăcinii primelor k frunze cu rădăcina celorlalte n−k [observat]. Prefixele de frunză și de nod diferă (separare de domeniu, previne confuzia nod/frunză) [observat]. Forma arborelui e determinată doar de n; n care nu e putere a lui doi este permis [observat].
5. Dovada de includere: pentru frunza cu indice m dintr-un arbore de n, lista hash-urilor frații pe drumul de la frunză la rădăcină, de jos în sus [observat]. Pentru n = 1 dovada este vidă și hash-ul frunzei este chiar rădăcina [observat].
6. Verificarea includerii reface rădăcina din hash-ul frunzei și dovadă, urmând pozițiile determinate de (indice, mărime) și acceptă doar dacă lista a fost consumată exact și rădăcina refăcută egală cu cea așteptată [observat]. Eșecuri: indice ≥ mărime; dovadă prea lungă (se termină spațiul) sau prea scurtă; rădăcină diferită [observat].
7. Dovada de consistență între mărimea veche m și cea nouă n, cu 0 < m < n: listă de hash-uri din care se poate reface atât rădăcina veche cât și cea nouă; are cel mult ceil(log2 n) + 1 elemente [observat, 2.1.4.1]. Când m este putere exactă a lui doi, rădăcina veche însăși servește ca prim element la verificare și nu e inclusă în dovadă [observat, 2.1.4.2].
8. Verificarea consistenței acceptă doar dacă ambele rădăcini refăcute coincid cu cele date și toată dovada a fost consumată [observat]. Eșecuri: dovadă vidă; dovadă care se epuizează devreme; oricare rădăcină diferită [observat].
9. Mărimi egale (m = n): generarea dă dovadă vidă, dar verificarea cere m < n și respinge dovada vidă; cazul se tratează comparând direct cele două rădăcini [observat]. m = 0: consistența nu este definită [observat].
10. Registrul pe colecție: după adăugare de fișiere, noua rădăcină plus dovada de consistență față de rădăcina publicată anterior arată că niciun fișier vechi nu s-a schimbat sau a fost scos [dedus din scopul 2.1.4].

## Formate / protocoale
Hash-uri ca șiruri hexazecimale în orice stocare proprie; ordinea frunzelor este ordinea de inserare, nu alfabetică (alfabetizarea ar schimba rădăcina) [dedus]. Codificarea conținutului frunzei (linie de text, cu sau fără sfârșit de linie) trebuie fixată de utilizator și păstrată; RFC-ul definește frunza ca octeți opaci [dedus].

## Cazuri limită și erori
- Mărime 0, 1, 2, 3 și puteri ale lui doi, plus un număr fără putere a lui doi (ex. 7, 11): toate cazurile structurale ale împărțirii în k și n−k.
- Indice sau mărimi în afara domeniului → eroare de intrare, nu „dovadă invalidă".
- Elemente de dovadă cu lungime diferită de a hash-ului → respingere [dedus din 2.1.1/4.1].
- Securitate: dovezile nu protejează împotriva schimbării simultane a fișierelor și a registrului dacă rădăcina nu a fost publicată separat [dedus].

## Teste de acceptanță
1. Arbore cu 0 frunze → rădăcina egală cu hash-ul șirului vid (pentru SHA-256, valoarea de la punctul 2).
2. Arbore cu 1 frunză: dovada de includere pentru indicele 0 este vidă; rădăcina = hash-ul frunzei cu prefix de frunză.
3. Arbore cu 2 frunze: dovada de includere pentru fiecare indice are exact un element (hash-ul celeilalte frunze).
4. Arbore cu 7 frunze (împărțire 4 + 3): pentru fiecare dintre cei 7 indici dovada generată se verifică; dacă se schimbă un octet al frunzei sau al unui element, verificarea eșuează; indice 7 → eșec.
5. Consistență 3 → 7: dovada generată se verifică cu ambele rădăcini; dovada are cel mult ceil(log2 7) + 1 = 4 elemente; dovadă vidă → eșec; rădăcina veche înlocuită cu alta → eșec.
6. Consistență 4 → 8 (m putere a lui doi): dovada nu conține rădăcina veche, verificarea reușește folosind-o ca prim element.
7. Consistență m = n: verificatorul de dovezi respinge dovada vidă, dar comparația directă a rădăcinilor dă „consistent" când sunt egale.
8. Toate perechile 0 < m < n ≤ 17: dovada generată trece verificarea; aceleași perechi cu un element al dovezii alterat eșuează.

## Necunoscute
- Valorile octeților-prefix (frunză/nod) nu au fost reținute din sursa citită; se citesc din 2.1.1 înainte de implementare.
- Registrul IANA de algoritmi nu a fost consultat.
- Rezumatul sursei descrie verificarea cu variabile de lucru; implementarea le re-derivează din definiția arborelui, iar testele 4-8 sunt singura garanție de corectitudine.

## Checklist de filtru
Spec: 02-merkle-dovezi   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (nu am verificat declarațiile IPR)
- [ ] Aprobată de mine înainte de predare
