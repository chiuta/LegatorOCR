# Întrebări rămase fără răspuns în specificații (sesiunea de implementare)

Fiecare punct: spec, ce lipsește, ce s-a făcut în cod până la răspuns.

## 01 bagit-validare
1. Secvențe procentuale în afara `%0A`, `%0D`, `%25` ([necunoscut], punctul 6): codul refuză linia (constatare „malformed"), nu ghicește decodarea.
2. Semantica „complet" cu preluări (`fetch.txt`) nefinalizate ([necunoscut], punctul 11): codul nu descarcă nimic și raportează fișierul listat dar absent ca „missing" (pachet incomplet); varianta opusă nu a fost aleasă.
3. Algoritm de manifest necunoscut (altul decât md5, sha1, sha256, sha512): spec-ul nu spune ce se întâmplă; codul raportează eroare „checksum" (nimic nu a fost verificat), deci pachetul nu e valid.
4. Spec-ul nu spune dacă un fișier din `data/` nelistat se raportează o dată per manifest sau o dată în total; codul îl raportează per manifest.
