// Mobile navigation
var navToggle = document.getElementById('navToggle');
var navLinks = document.getElementById('navLinks');

navToggle.addEventListener('click', function () {
    var open = navLinks.classList.toggle('open');
    navToggle.setAttribute('aria-expanded', open ? 'true' : 'false');
});

navLinks.querySelectorAll('a').forEach(function (link) {
    link.addEventListener('click', function () {
        navLinks.classList.remove('open');
        navToggle.setAttribute('aria-expanded', 'false');
    });
});

// Get-started tabs
document.querySelectorAll('.tab').forEach(function (tab) {
    tab.addEventListener('click', function () {
        document.querySelectorAll('.tab').forEach(function (t) {
            t.classList.toggle('active', t === tab);
        });
        document.querySelectorAll('.pane').forEach(function (pane) {
            pane.classList.toggle('active', pane.id === 'pane-' + tab.dataset.pane);
        });
    });
});

// Copy buttons
document.querySelectorAll('.copy').forEach(function (btn) {
    btn.addEventListener('click', function () {
        navigator.clipboard.writeText(btn.dataset.copy).then(function () {
            btn.classList.add('copied');
            btn.textContent = 'copied';
            setTimeout(function () {
                btn.classList.remove('copied');
                btn.textContent = 'copy';
            }, 1600);
        });
    });
});
