(() => {
  const elForm = document.querySelector("form.search-form");
  if (!elForm) {
    return;
  }

  const elQ = elForm.querySelector("#q");
  const elRepo = elForm.querySelector("#repo");
  const elScope = elForm.querySelector("#scope");

  // `query` (everything) and `name` (name only) are mutually exclusive params,
  // so the scope picks which one the search input submits as.
  elScope.addEventListener("change", (e) => (elQ.name = e.target.value));

  // The repo is a path segment (/repos/$repo) and not a query param, so the
  // selection only rewrites the form's action.
  const root = elForm.action.replace(/\/repos\/.*$/, "");
  const selectRepo = (slug) => {
    elForm.action = `${root}/repos/${slug}`;
    localStorage.setItem("repo", slug);
  };

  // On the landing page, start with the repo that was last searched.
  const last = localStorage.getItem("repo");
  if (document.body.classList.contains("index") && last && elRepo.querySelector(`option[value="${CSS.escape(last)}"]`)) {
    elRepo.value = last;
  }
  selectRepo(elRepo.value);

  elRepo.addEventListener("change", (e) => selectRepo(e.target.value));

  // Keep submitted URLs clean by leaving out the empty (advanced) fields.
  // Disabled fields are excluded from the submission, and are re-enabled
  // immediately after, by which time the form data has been serialized.
  elForm.addEventListener("submit", () => {
    const empty = Array.from(elForm.querySelectorAll("input, select")).filter((el) => !el.value.trim());
    empty.forEach((el) => (el.disabled = true));
    setTimeout(() => empty.forEach((el) => (el.disabled = false)), 0);
  });

  // On / press, focus the search input.
  document.addEventListener("keydown", (e) => {
    if (e.key !== "/" || e.target.matches("input, select, textarea")) {
      return;
    }

    e.preventDefault();
    elQ.focus();
    elQ.select();
  });
})();
