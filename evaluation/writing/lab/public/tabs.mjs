export function attachLabTabs(document, names = ['prediction', 'spelling', 'lookup']) {
  const tabs = names.filter(name => document.getElementById(`${name}-tab`));
  const select = name => {
    for (const id of tabs) {
      const button = document.getElementById(`${id}-tab`);
      button.setAttribute('aria-selected', String(id === name));
      button.tabIndex = id === name ? 0 : -1;
      document.getElementById(`${id}-panel`).hidden = id !== name;
    }
  };
  for (const [index, name] of tabs.entries()) {
    const button = document.getElementById(`${name}-tab`);
    button.onclick = () => select(name);
    button.onkeydown = event => {
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const next = event.key === 'Home' ? tabs[0] : event.key === 'End' ? tabs.at(-1)
        : tabs[(index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length];
      select(next);
      document.getElementById(`${next}-tab`).focus();
    };
  }
}
