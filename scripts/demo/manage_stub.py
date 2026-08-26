#!/usr/bin/env python3
"""Faux serveur Inklura Manage — données de démonstration uniquement.

Sert les routes que ShellDeck interroge (`/api/manage/shelldeck/...`) avec un
jeu de données entièrement inventé : aucun client, aucun hôte et aucun ticket
réel n'y figure. Il existe pour deux usages :

  * produire les captures de la documentation sans exposer de données ;
  * ouvrir les surfaces Utilisateur et Support sur une machine qui n'a pas
    accès au vrai portail.

Utilisation :  python3 scripts/demo/manage_stub.py [--port 8899]

Le serveur ne fait aucune écriture : les actions (répondre, changer un statut)
renvoient un succès sans rien conserver.
"""

import argparse
import json
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

NOW = time.time() * 1000.0
MIN = 60_000.0
H = 60 * MIN

AGENT = {"name": "Équipe de démonstration", "email": "support@exemple.test"}


def ticket(tid, subject, status, priority, contact, minutes_ago, preview, unread, count):
    return {
        "id": tid, "channel": "livechat", "subject": subject,
        "contact": {"name": contact, "email": f"{contact.split()[0].lower()}@exemple.test"},
        "status": status, "unread": unread,
        "assignee": AGENT["email"] if tid == "t-1002" else "",
        "last_at": NOW - minutes_ago * MIN, "msg_count": count,
        "last_preview": preview, "priority": priority,
    }


TICKETS = [
    ticket("t-1001", "Le site est lent depuis la mise à jour", "open", "high",
           "Claire Meunier", 12, "C'est surtout la page catalogue qui rame.", True, 4),
    ticket("t-1002", "Erreur 500 sur le formulaire de contact", "pending", "normal",
           "Yanis Berger", 95, "Je vous envoie la capture demandée.", False, 6),
    ticket("t-1003", "Demande de restauration d'une sauvegarde", "open", "urgent",
           "Sophie Renard", 3, "Il nous faut la version d'avant-hier.", True, 2),
    ticket("t-1004", "Question sur la facturation annuelle", "closed", "low",
           "Marc Oliveira", 26 * 60, "Parfait, merci pour la précision.", False, 3),
]

def thread(contact, exchanges):
    """Construit un fil : (auteur, texte, minutes) — `agent` ou le contact."""
    out = []
    for who, text, mins, kind in exchanges:
        out.append({
            "from": "agent" if who == "agent" else "contact",
            "text": text, "at": NOW - mins * MIN,
            "name": AGENT["name"] if who == "agent" else contact,
            "kind": kind, "channel": "internal" if kind == "note" else "livechat",
        })
    return out


MESSAGES = {
    "t-1002": thread("Yanis Berger", [
        ("contact", "Le formulaire de contact renvoie une erreur depuis ce matin.", 180, "message"),
        ("agent", "Merci, je regarde les journaux du serveur.", 170, "message"),
        ("agent", "Erreur côté service d'envoi, ticket ouvert chez l'hébergeur.", 150, "note"),
        ("contact", "Je vous envoie la capture demandée.", 95, "message"),
    ]),
    "t-1003": thread("Sophie Renard", [
        ("contact", "Il nous faut la version d'avant-hier, on a perdu le catalogue.", 8, "message"),
        ("agent", "Restauration lancée, je vous confirme dans quinze minutes.", 3, "message"),
    ]),
    "t-1004": thread("Marc Oliveira", [
        ("contact", "La facture annuelle couvre-t-elle les deux environnements ?", 30 * 60, "message"),
        ("agent", "Oui, production et préproduction sont incluses.", 28 * 60, "message"),
        ("contact", "Parfait, merci pour la précision.", 26 * 60, "message"),
    ]),
    "t-1001": [
        {"from": "contact", "text": "Bonjour, depuis hier le site met une éternité à charger.",
         "at": NOW - 40 * MIN, "name": "Claire Meunier", "kind": "message", "channel": "livechat"},
        {"from": "agent", "text": "Bonjour Claire, je regarde ça tout de suite.",
         "at": NOW - 35 * MIN, "name": AGENT["name"], "kind": "message", "channel": "livechat"},
        {"from": "agent", "text": "Cache applicatif purgé, je surveille les temps de réponse.",
         "at": NOW - 20 * MIN, "name": AGENT["name"], "kind": "note", "channel": "internal"},
        {"from": "contact", "text": "C'est surtout la page catalogue qui rame.",
         "at": NOW - 12 * MIN, "name": "Claire Meunier", "kind": "message", "channel": "livechat"},
    ]
}


def issue(iid, title, status, priority, source, by, hours_ago, comments, body):
    return {
        "id": iid, "tenant_id": "demo", "tenant_name": "Démonstration",
        "site_id": "site-boutique", "site_label": "Boutique de démonstration",
        "title": title, "status": status, "priority": priority, "source": source,
        "requested_by": by, "assignee": "", "comment_count": comments,
        "attachment_count": 0, "job_count": 0,
        "created_at": NOW - hours_ago * H, "updated_at": NOW - (hours_ago / 2) * H,
        "body": body,
    }


# Deux demandes portent le nom du compte de démonstration : sans elles, le mode
# Utilisateur affiche une liste vide, car « Mes demandes » ne retient que celles
# dont `requested_by` correspond au compte connecté.
ISSUES = [
    issue("i-2001", "Ajouter un filtre par marque sur le catalogue", "open", "normal",
          "user", "Claire Meunier", 30, 2,
          "Les clients cherchent souvent par marque et doivent faire défiler toute la liste."),
    issue("i-2002", "Le formulaire de contact ne fonctionne plus", "in_progress", "high",
          "user", "Yanis Berger", 6, 4,
          "Depuis ce matin, l'envoi affiche une erreur 500. Reproduit sur Firefox et Chrome."),
    issue("i-2003", "Passer la page d'accueil en deux colonnes sur mobile", "done", "low",
          "support", "Sophie Renard", 72, 3,
          "Converti depuis un ticket : la mise en page mobile tasse les visuels."),
    issue("i-2004", "Prévoir une bannière pour les soldes d'été", "open", "normal",
          "user", AGENT["name"], 20, 2,
          "Un visuel pleine largeur sur la page d'accueil, du 1er au 30 juin."),
    issue("i-2005", "Certificat expiré sur la préproduction", "in_progress", "urgent",
          "user", AGENT["name"], 3, 1,
          "Le navigateur affiche un avertissement en ouvrant preprod.boutique.exemple.test."),
]

# Les commentaires voyagent *dans* l'objet demande (`issue.comments`), pas à
# côté : le client ne lit que cette forme.
ISSUE_COMMENTS = {
    "i-2004": [
        {"id": "c-1", "author": AGENT["name"], "kind": "comment",
         "body": "Voici la maquette validée par la direction artistique.",
         "at": NOW - 18 * H},
        {"id": "c-2", "author": "Équipe Inklura", "kind": "comment",
         "body": "Bien reçu, nous planifions la mise en ligne pour la semaine "
                 "prochaine. Le visuel sera en place le 28 mai au plus tard.",
         "at": NOW - 10 * H},
    ],
    "i-2005": [
        {"id": "c-3", "author": "Équipe Inklura", "kind": "comment",
         "body": "Renouvellement lancé, le certificat sera actif d'ici une heure.",
         "at": NOW - 1 * H},
    ],
}


def owned_by_account(iss):
    """Les demandes du compte connecté, au sens du filtre `mine=1` de Manage."""
    return iss["requested_by"] in (AGENT["name"], AGENT["email"])


def body_for(path):
    if "auth" in path:
        return {"ok": True, "token": "demo-token", "label": "Démonstration",
                # `user.roles` est le sac de rôles que le client lit pour la
                # carte « Rôles » ; `role`/`roleNames` sont les formes héritées.
                "user": {"name": AGENT["name"], "email": AGENT["email"],
                         "role": "superadmin",
                         "roles": ["superadmin", "inklura_support"],
                         "roleNames": ["superadmin", "inklura_support"]},
                "is_superadmin": True, "is_admin": True, "is_inklura_support": True,
                "roles": ["superadmin", "inklura_support"]}

    if "support" in path:
        if "action=ticket" in path:
            tid = path.split("id=")[-1].split("&")[0]
            found = next((t for t in TICKETS if t["id"] == tid), TICKETS[0])
            return {"ok": True, "ticket": dict(found, messages=MESSAGES.get(tid, []))}
        if "action=agents" in path:
            return {"ok": True, "agents": [AGENT]}
        if "action=list" in path or path.endswith("support"):
            # S-05 regression fixture: older/partial servers may omit `all`.
            # ShellDeck must use the received list as the total's lower bound.
            return {"ok": True, "tickets": TICKETS,
                    "counts": {"unassigned": 3, "mine": 1,
                               "open": 2, "pending": 1, "breaching": 1,
                               "closed": 1},
                    "me": AGENT}
        return {"ok": True}

    if "issues" in path:
        if "id=" in path:
            iid = path.split("id=")[-1].split("&")[0]
            found = next((i for i in ISSUES if i["id"] == iid), ISSUES[0])
            return {"ok": True,
                    "issue": dict(found, comments=ISSUE_COMMENTS.get(iid, [])),
                    "staff": True}
        # `mine=1` est envoyé par le mode Utilisateur : le respecter évite de
        # laisser croire que le client voit les demandes des autres.
        listed = [i for i in ISSUES if owned_by_account(i)] if "mine=1" in path else ISSUES
        if "mine=1" in path:
            # U-05 regression fixture: Manage may prove ownership with its
            # internal actor id while presenting a composite author label.
            # ShellDeck must trust the owner-scoped response instead of
            # filtering the same rows again by exact display-name equality.
            listed = [
                dict(
                    issue,
                    requested_by=f'{AGENT["name"]} <{AGENT["email"]}>',
                )
                for issue in listed
            ]
        count_universe = listed
        if "status=" in path:
            selected_status = path.split("status=")[-1].split("&")[0]
            listed = [issue for issue in listed if issue.get("status") == selected_status]
        counts = {"all": len(count_universe), "open": 0, "triaging": 0,
                  "in_progress": 0, "blocked": 0, "done": 0, "closed": 0}
        for issue in count_universe:
            status = issue.get("status", "")
            if status in counts:
                counts[status] += 1
        return {"ok": True, "issues": listed, "total": len(listed), "counts": counts,
                "staff": True, "instances": []}

    if "sites" in path:
        return {"ok": True, "manage_origin": "https://demo.exemple.test",
                "sites": [{"site_id": "site-boutique",
                           "label": "Boutique de démonstration",
                           "host": "boutique.exemple.test",
                           "tenant_id": "demo", "tenant_name": "Démonstration"},
                          {"site_id": "site-atelier",
                           "label": "Atelier de démonstration",
                           "host": "atelier.exemple.test",
                           "tenant_id": "demo", "tenant_name": "Démonstration"}],
                "areas": []}

    return {"ok": True}


class Handler(BaseHTTPRequestHandler):
    def respond(self):
        data = json.dumps(body_for(self.path)).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = respond

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8899)
    port = parser.parse_args().port
    print(f"Faux Manage sur http://127.0.0.1:{port} — données fictives uniquement")
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
