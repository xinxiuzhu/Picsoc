import React from 'react';
import ReactDOM from 'react-dom/client';
import './i18n';
import AuthGate from './AuthGate';
import './styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <AuthGate />
  </React.StrictMode>,
);
