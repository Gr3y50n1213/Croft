// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
//
// The enrolment page (T24). The link's token is in the URL fragment, which browsers never send to
// a server; it is read once and removed from the address bar. Only textContent is ever set (the
// page enforces Trusted Types, so markup from data cannot run).
"use strict";

(function () {
  const token = location.hash.slice(1);
  history.replaceState(null, "", location.pathname);

  const $ = (id) => document.getElementById(id);
  const status = (text) => { $("status").textContent = text; };

  const fromB64 = (s) => {
    const b = atob(s.replace(/-/g, "+").replace(/_/g, "/"));
    return Uint8Array.from(b, (c) => c.charCodeAt(0)).buffer;
  };
  const toB64 = (buf) => {
    let s = "";
    for (const b of new Uint8Array(buf)) s += String.fromCharCode(b);
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  };

  async function post(path, body) {
    const res = await fetch(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      credentials: "omit",
      cache: "no-store",
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.error || "server");
    return data;
  }

  function explain(e) {
    switch (e && e.message) {
      case "invalid": return "This link has expired or was already used. Ask your admin for a new one.";
      case "rate_limited": return "Too many tries. Wait a few minutes and try again.";
      case "conflict": return "That passkey is already registered. Try another device or security key.";
      case "NotAllowedError": return "The passkey was not made (cancelled or timed out). Try again.";
      default: return "Something went wrong. Try again, or ask your admin for a new link.";
    }
  }

  let ceremony = null;
  let options = null;

  async function start() {
    if (!token) { status("This page needs the link your admin gave you."); return; }
    if (!window.PublicKeyCredential) { status("This browser cannot make passkeys. Try another browser or device."); return; }
    try {
      const r = await post("/api/enrol/start", { token });
      ceremony = r.ceremony;
      options = r.options.publicKey;
      $("who").textContent = "Setting up sign-in for " + r.user + ".";
      $("who").hidden = false;
      $("make-passkey").disabled = false;
    } catch (e) {
      status(explain(e));
    }
  }

  async function makePasskey() {
    $("make-passkey").disabled = true;
    status("Follow your device's steps…");
    try {
      const publicKey = Object.assign({}, options, {
        challenge: fromB64(options.challenge),
        user: Object.assign({}, options.user, { id: fromB64(options.user.id) }),
        excludeCredentials: (options.excludeCredentials || []).map((c) => Object.assign({}, c, { id: fromB64(c.id) })),
      });
      const cred = await navigator.credentials.create({ publicKey });
      const credential = {
        id: cred.id,
        rawId: toB64(cred.rawId),
        type: cred.type,
        response: {
          attestationObject: toB64(cred.response.attestationObject),
          clientDataJSON: toB64(cred.response.clientDataJSON),
          transports: cred.response.getTransports ? cred.response.getTransports() : undefined,
        },
        extensions: cred.getClientExtensionResults(),
      };
      const done = await post("/api/enrol/finish", { token, ceremony, credential });
      showBackup(done);
    } catch (e) {
      const why = e && e.name === "NotAllowedError" ? { message: "NotAllowedError" } : e;
      status(explain(why));
      if (why && why.message !== "invalid") {
        // Each try spends its challenge: fetch a new one, keeping the message shown.
        const shown = $("status").textContent;
        await start();
        if (!$("make-passkey").disabled) status(shown);
      }
    }
  }

  function showBackup(done) {
    $("step-passkey").hidden = true;
    $("secret").textContent = done.totp.secret;
    const list = $("codes");
    for (const c of done.recovery_codes) {
      const li = document.createElement("li");
      const code = document.createElement("code");
      code.textContent = c;
      li.appendChild(code);
      list.appendChild(li);
    }
    drawQr($("qr"), done.totp.qr);
    $("step-backup").hidden = false;
  }

  function drawQr(canvas, rows) {
    const n = rows.length;
    const quiet = 4;
    const scale = Math.floor(canvas.width / (n + 2 * quiet));
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = "#000";
    rows.forEach((row, y) => {
      for (let x = 0; x < row.length; x++) {
        if (row[x] === "1") ctx.fillRect((x + quiet) * scale, (y + quiet) * scale, scale, scale);
      }
    });
  }

  $("make-passkey").addEventListener("click", makePasskey);
  start();
})();
