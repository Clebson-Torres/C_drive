import React from "react";

function SavedriveMark() {
  return (
    <span className="inline-flex h-9 w-9 items-center justify-center rounded-2xl bg-gradient-to-br from-sky-500 via-cyan-400 to-blue-700 text-lg font-bold text-white shadow-lg shadow-sky-500/30">
      S
    </span>
  );
}

export default function LoginScreen({ t, authState, message, phone, code, password, busyAction, onPhoneChange, onCodeChange, onPasswordChange, onStart, onVerifyCode, onVerifyPassword }) {
  const cards = t("login.cards");
  return (
    <section id="authScreen" className="mx-auto flex min-h-screen w-full max-w-6xl items-center justify-center px-6 py-10">
      <div className="grid w-full gap-10 lg:grid-cols-[1.1fr_0.9fr]">
        <div className="hidden rounded-[36px] border border-white/60 bg-white/45 p-10 shadow-glass backdrop-blur-xl lg:flex lg:flex-col lg:justify-between">
          <div className="space-y-6">
            <span className="inline-flex items-center gap-3 rounded-full border border-sky-200 bg-white/80 px-4 py-2 text-xs font-semibold uppercase tracking-[0.32em] text-sky-700">
              <SavedriveMark />
              {t("login.badge")}
            </span>
            <h1 className="max-w-xl text-5xl font-semibold tracking-tight text-slate-950">{t("login.heroTitle")}</h1>
            <p className="max-w-2xl text-lg text-slate-700">{t("login.heroBody")}</p>
          </div>
          <div className="grid gap-4 md:grid-cols-3">
            {cards.map(([title, body]) => (
              <div key={title} className="rounded-3xl border border-sky-100/90 bg-white/70 p-5 shadow-sm">
                <div className="text-sm font-semibold text-slate-900">{title}</div>
                <p className="mt-2 text-sm leading-6 text-slate-600">{body}</p>
              </div>
            ))}
          </div>
        </div>
        <div className="rounded-[36px] border border-white/60 bg-white/55 p-8 shadow-glass backdrop-blur-xl sm:p-10">
          <div className="mb-8 space-y-3">
            <p className="text-sm font-semibold uppercase tracking-[0.28em] text-sky-700">{t("login.section")}</p>
            <h2 className="text-3xl font-semibold text-slate-950">{t("login.title")}</h2>
            <p className="text-sm leading-6 text-slate-600">{t("login.body")}</p>
          </div>
          {message ? <div id="authMessage" className={`mb-6 rounded-2xl border px-4 py-3 text-sm ${message.kind === "ok" ? "border-emerald-200 bg-emerald-50 text-emerald-700" : "border-rose-200 bg-rose-50 text-rose-700"}`}>{message.text}</div> : <div id="authMessage" className="hidden" />}
          <div id="authFormStart" className={authState === "LoggedOut" ? "space-y-4" : "hidden"}>
            <label className="block text-sm font-medium text-slate-700">{t("login.phone")}
              <input id="inputPhone" value={phone} onChange={(event) => onPhoneChange(event.target.value)} placeholder="+5511999999999" className="mt-2 w-full rounded-2xl border border-slate-200 bg-white/90 px-4 py-3 text-slate-900 outline-none transition focus:border-sky-400 focus:ring-4 focus:ring-sky-100" />
            </label>
            <button id="btnAuthStart" type="button" disabled={busyAction === "start"} onClick={onStart} className="w-full rounded-2xl bg-gradient-to-r from-sky-600 to-blue-700 px-5 py-3 text-sm font-semibold text-white shadow-lg shadow-sky-500/25 transition hover:brightness-105 disabled:cursor-wait disabled:opacity-70">{busyAction === "start" ? t("login.continueLoading") : t("login.continue")}</button>
          </div>
          <div id="authFormCode" className={authState === "AwaitingCode" ? "space-y-4" : "hidden"}>
            <label className="block text-sm font-medium text-slate-700">{t("login.code")}
              <input id="inputCode" value={code} onChange={(event) => onCodeChange(event.target.value)} placeholder="12345" className="mt-2 w-full rounded-2xl border border-slate-200 bg-white/90 px-4 py-3 text-slate-900 outline-none transition focus:border-sky-400 focus:ring-4 focus:ring-sky-100" />
            </label>
            <button id="btnAuthCode" type="button" disabled={busyAction === "code"} onClick={onVerifyCode} className="w-full rounded-2xl bg-gradient-to-r from-sky-600 to-blue-700 px-5 py-3 text-sm font-semibold text-white shadow-lg shadow-sky-500/25 transition hover:brightness-105 disabled:cursor-wait disabled:opacity-70">{busyAction === "code" ? t("login.verifyCodeLoading") : t("login.verifyCode")}</button>
          </div>
          <div id="authFormPassword" className={authState === "AwaitingPassword" ? "space-y-4" : "hidden"}>
            <label className="block text-sm font-medium text-slate-700">{t("login.password")}
              <input id="inputPassword" type="password" value={password} onChange={(event) => onPasswordChange(event.target.value)} placeholder="••••••••" className="mt-2 w-full rounded-2xl border border-slate-200 bg-white/90 px-4 py-3 text-slate-900 outline-none transition focus:border-sky-400 focus:ring-4 focus:ring-sky-100" />
            </label>
            <button id="btnAuthPassword" type="button" disabled={busyAction === "password"} onClick={onVerifyPassword} className="w-full rounded-2xl bg-gradient-to-r from-sky-600 to-blue-700 px-5 py-3 text-sm font-semibold text-white shadow-lg shadow-sky-500/25 transition hover:brightness-105 disabled:cursor-wait disabled:opacity-70">{busyAction === "password" ? t("login.verifyPasswordLoading") : t("login.verifyPassword")}</button>
          </div>
        </div>
      </div>
    </section>
  );
}
