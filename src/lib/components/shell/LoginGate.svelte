<script lang="ts">
  import { licenseStatus } from "../../../stores/session";
  import {
    licenseRequestOtp,
    licenseVerifyOtp,
  } from "../../../ipc/commands";
  import { formatAppError } from "../../../ipc/types";
  import logoIcon from "../../icons/logo.png";

  const inTauri = typeof window !== "undefined" && Boolean((window as any).__TAURI_INTERNALS__);

  let step = $state<"email" | "code">("email");
  let email = $state("");
  let code = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let resendAt = $state(0);
  let now = $state(Date.now());
  let emailEl = $state<HTMLInputElement | null>(null);
  let codeEl = $state<HTMLInputElement | null>(null);

  const needsGate = $derived(false);
  const resendWait = $derived(Math.max(0, Math.ceil((resendAt - now) / 1000)));

  $effect(() => {
    if (!needsGate) return;
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });

  $effect(() => {
    if (!needsGate) return;
    const el = step === "email" ? emailEl : codeEl;
    requestAnimationFrame(() => el?.focus());
  });

  function friendly(e: unknown): string {
    return formatAppError(e).replace(/^Internal:\s*/i, "");
  }

  async function sendCode() {
    if (busy) return;
    const em = email.trim();
    if (!em) {
      error = "Enter your email.";
      return;
    }
    busy = true;
    error = null;
    try {
      await licenseRequestOtp(em);
      email = em.toLowerCase();
      step = "code";
      code = "";
      resendAt = Date.now() + 45_000;
    } catch (e) {
      error = friendly(e);
    } finally {
      busy = false;
    }
  }

  async function verify() {
    if (busy) return;
    const digits = code.replace(/\D/g, "");
    if (digits.length !== 6) {
      error = "Enter the 6-digit code from your email.";
      return;
    }
    busy = true;
    error = null;
    try {
      const result = await licenseVerifyOtp(email, digits);
      licenseStatus.set(result);
    } catch (e) {
      error = friendly(e);
    } finally {
      busy = false;
    }
  }

  function onCodeInput(e: Event) {
    const v = (e.currentTarget as HTMLInputElement).value.replace(/\D/g, "").slice(0, 6);
    code = v;
    if (v.length === 6 && !busy) void verify();
  }

  function backToEmail() {
    step = "email";
    code = "";
    error = null;
  }

  function onEmailKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void sendCode();
    }
  }

  function onCodeKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      void verify();
    }
  }
</script>

{#if needsGate}
  <div class="gate" role="dialog" aria-modal="true" aria-labelledby="login-title">
    <div class="card">
      <img src={logoIcon} alt="" class="logo" width="28" height="28" />
      <h1 id="login-title">Sign in to MeraRAW</h1>

      {#if step === "email"}
        <p class="lede">Enter the email you want this copy linked to. We’ll send a 6-digit code.</p>
        <label class="field">
          <span>Email</span>
          <input
            bind:this={emailEl}
            type="email"
            autocomplete="email"
            inputmode="email"
            placeholder="you@studio.com"
            bind:value={email}
            onkeydown={onEmailKey}
            disabled={busy}
          />
        </label>
        {#if error}
          <p class="err" role="alert">{error}</p>
        {/if}
        <button type="button" class="go" disabled={busy} onclick={() => void sendCode()}>
          {busy ? "Sending…" : "Send code"}
        </button>
      {:else}
        <p class="lede">
          We sent a 6-digit code to <strong>{email}</strong>. It may take a few seconds.
        </p>
        <label class="field">
          <span>Code</span>
          <input
            bind:this={codeEl}
            class="code"
            type="text"
            inputmode="numeric"
            autocomplete="one-time-code"
            maxlength="6"
            placeholder="••••••"
            value={code}
            oninput={onCodeInput}
            onkeydown={onCodeKey}
            disabled={busy}
          />
        </label>
        {#if error}
          <p class="err" role="alert">{error}</p>
        {/if}
        <button type="button" class="go" disabled={busy || code.replace(/\D/g, "").length !== 6} onclick={() => void verify()}>
          {busy ? "Checking…" : "Continue"}
        </button>
        <div class="row">
          <button type="button" class="link" disabled={busy} onclick={backToEmail}>Use a different email</button>
          <button
            type="button"
            class="link"
            disabled={busy || resendWait > 0}
            onclick={() => void sendCode()}
          >
            {resendWait > 0 ? `Resend in ${resendWait}s` : "Resend code"}
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .gate {
    position: absolute;
    inset: 0;
    z-index: 400;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-bg);
    padding: 24px;
  }
  .card {
    width: 100%;
    max-width: 380px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .logo {
    width: 28px;
    height: 28px;
    border-radius: 6px;
  }
  h1 {
    margin: 4px 0 0;
    font-size: 18px;
    font-weight: 560;
    color: var(--color-fg);
  }
  .lede {
    margin: 0;
    font-size: var(--text-ui);
    line-height: 1.45;
    color: var(--color-subtle);
  }
  .lede strong {
    color: var(--color-fg);
    font-weight: 500;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  .field span {
    font-size: var(--text-group);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--color-subtle);
  }
  .field input {
    height: 36px;
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    background: var(--color-sunken);
    color: var(--color-fg);
    font-size: var(--text-ui);
    padding: 0 12px;
  }
  .field input:focus {
    outline: none;
    border-color: var(--color-fg);
  }
  .field input:disabled {
    opacity: 0.6;
  }
  .code {
    font-family: var(--font-mono);
    font-size: 22px;
    letter-spacing: 0.35em;
    text-align: center;
    padding-right: 0;
  }
  .err {
    margin: 0;
    font-size: var(--text-ui);
    color: var(--color-warn);
  }
  .go {
    margin-top: 4px;
    height: 36px;
    border: 0;
    border-radius: 8px;
    background: var(--color-fg);
    color: var(--color-panel);
    font-size: var(--text-ui);
    font-weight: 500;
    cursor: pointer;
  }
  .go:hover:not(:disabled) {
    opacity: 0.9;
  }
  .go:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font-size: var(--text-ui);
    color: var(--color-subtle);
    cursor: pointer;
  }
  .link:hover:not(:disabled) {
    color: var(--color-fg);
  }
  .link:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>