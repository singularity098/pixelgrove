import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

export default mount(App, { target: document.getElementById('app')! });

// Makes PixelGrove installable as an app (and quicker to open). Browsers only allow
// service workers over HTTPS or on localhost.
if ('serviceWorker' in navigator && import.meta.env.PROD) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js').catch(() => {});
  });
}
