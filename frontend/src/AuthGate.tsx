import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowRight, Images, LoaderCircle, LockKeyhole, RefreshCw } from 'lucide-react';
import type { ReactNode } from 'react';
import App from './App';
import { api, ApiError, AUTH_REQUIRED_EVENT, errorMessage } from './api';
import { LanguageMenu } from './LanguageMenu';

interface AuthStatus {
  password_required: boolean;
  authenticated: boolean;
}

function AuthLayout({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  return <main className="auth-shell"><header className="auth-header"><a className="auth-brand" href="/" aria-label="Picsoc"><span className="brand-mark"><Images size={23} strokeWidth={1.8} /></span><span>Picsoc</span></a><LanguageMenu /></header><section className="auth-card">{children}</section><footer className="auth-footer">{t('app.auth.footer')}</footer></main>;
}

function Login({ onAuthenticated }: { onAuthenticated: (status: AuthStatus) => void }) {
  const { t } = useTranslation();
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const passwordInput = useRef<HTMLInputElement>(null);
  return <AuthLayout><span className="auth-symbol"><LockKeyhole size={25} strokeWidth={1.7} /></span><h1>{t('app.auth.title')}</h1><p className="auth-description">{t('app.auth.description')}</p><form onSubmit={event => {
    event.preventDefault();
    if (busy || !password) return;
    passwordInput.current?.focus();
    setBusy(true); setError('');
    void api<AuthStatus>('/api/auth/login', { method: 'POST', body: JSON.stringify({ password }) }).then(status => {
      if (!status.authenticated) throw new Error(t('app.auth.loginFailed'));
      setPassword(''); onAuthenticated(status);
    }).catch(cause => {
      setError(cause instanceof ApiError && cause.status === 401 ? t('app.auth.wrongPassword') : errorMessage(cause));
    }).finally(() => setBusy(false));
  }}>
    <div className="auth-account"><span>{t('app.auth.account')}</span><strong>picsoc</strong></div><input type="hidden" name="username" autoComplete="username" value="picsoc" />
    <label className="auth-password-label" htmlFor="login-password">{t('app.auth.password')}</label><input ref={passwordInput} id="login-password" className="auth-password" name="password" type="password" value={password} onChange={event => { setPassword(event.target.value); setError(''); }} autoComplete="current-password" placeholder={t('app.auth.passwordPlaceholder')} required readOnly={busy} autoFocus aria-invalid={Boolean(error)} aria-describedby={error ? 'login-error' : undefined} />
    {error && <p id="login-error" className="inline-error" role="alert">{error}</p>}
    <button className="button primary auth-submit" type="submit" disabled={busy || !password}>{busy ? <LoaderCircle size={17} className="spin" /> : <ArrowRight size={17} />}{t(busy ? 'app.auth.signingIn' : 'app.auth.signIn')}</button>
  </form></AuthLayout>;
}

export default function AuthGate() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setError('');
    void api<AuthStatus>('/api/auth/status', { signal: controller.signal, cache: 'no-store' }).then(result => {
      if (!controller.signal.aborted) setStatus(result);
    }).catch(cause => { if (!controller.signal.aborted) setError(errorMessage(cause)); });
    return () => controller.abort();
  }, [attempt]);
  useEffect(() => {
    const expired = () => { setError(''); setStatus({ password_required: true, authenticated: false }); };
    window.addEventListener(AUTH_REQUIRED_EVENT, expired);
    return () => window.removeEventListener(AUTH_REQUIRED_EVENT, expired);
  }, []);
  const logout = useCallback(async () => {
    const result = await api<AuthStatus>('/api/auth/logout', { method: 'POST', body: '{}' });
    setStatus(result);
  }, []);

  if (!status) return <AuthLayout><span className="auth-symbol">{error ? <RefreshCw size={24} /> : <LoaderCircle size={24} className="spin" />}</span><h1>{t(error ? 'app.auth.connectionTitle' : 'app.auth.loadingTitle')}</h1>{error ? <><p className="auth-description auth-error" role="alert">{error}</p><button className="button primary auth-submit" onClick={() => setAttempt(previous => previous + 1)}><RefreshCw size={16} />{t('app.retry')}</button></> : <p className="auth-description" role="status">{t('app.auth.loadingDescription')}</p>}</AuthLayout>;
  if (status.password_required && !status.authenticated) return <Login onAuthenticated={setStatus} />;
  return <App onLogout={status.password_required ? logout : undefined} />;
}
