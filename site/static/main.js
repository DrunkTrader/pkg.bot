(() => {
  document.querySelectorAll("form.search-form").forEach((elForm) => {
    const elQ = elForm.querySelector("[data-search-query]");
    const elRepo = elForm.querySelector("[data-search-repo]");
    const elScope = elForm.querySelector("[data-search-scope]");

    elScope.addEventListener("change", (e) => (elQ.name = e.target.value));

    // The repo search is a URI (/repos/$repo). Redirect on search.
    const root = elForm.action.replace(/\/repos\/.*$/, "");
    const selectRepo = (slug) => {
      elForm.action = `${root}/repos/${slug}`;
      localStorage.setItem("repo", slug);
    };

    // Remember the last searched repo.
    const last = localStorage.getItem("repo");
    if (document.body.classList.contains("index") && last && elRepo.querySelector(`option[value="${CSS.escape(last)}"]`)) {
      elRepo.value = last;
    }
    selectRepo(elRepo.value);
    elRepo.addEventListener("change", (e) => selectRepo(e.target.value));

    // Keep search URLs clean by excluding empty search params.
    elForm.addEventListener("submit", () => {
      const empty = Array.from(elForm.querySelectorAll("input, select")).filter((el) => !el.value.trim());
      empty.forEach((el) => (el.disabled = true));
      setTimeout(() => empty.forEach((el) => (el.disabled = false)), 0);
    });

    // On / press, focus the search input.
    document.addEventListener("keydown", (e) => {
      if (elForm !== document.querySelector("form.search-form") || e.key !== "/" || e.target.matches("input, select, textarea")) {
        return;
      }

      e.preventDefault();
      elQ.focus();
      elQ.select();
    });
  });
})();
