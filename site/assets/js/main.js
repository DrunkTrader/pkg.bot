import "@knadh/oat";
import { autocomp } from "@knadh/autocomp";

(() => {
  const savedRepo = (() => {
    const value = localStorage.getItem("repo");
    if (!value) return null;
    try {
      const repo = JSON.parse(value);
      return repo && typeof repo.slug === "string" && typeof repo.name === "string" ? repo : null;
    } catch {
      return { slug: value, name: null };
    }
  })();

  document.querySelectorAll("[data-search-form]").forEach((elForm) => {
    const elQ = elForm.querySelector("[data-search-query]");
    const elRepo = elForm.querySelector("[data-search-repo]");
    const elScope = elForm.querySelector("[data-search-scope]");
    const isAdvanced = elForm.classList.contains("advanced-search");

    if (elScope) {
      elScope.addEventListener("change", (e) => (elQ.name = e.target.value));
    }

    // The repo search is a URI (/repos/$repo). Redirect on search.
    const root = elForm.action.replace(/\/repos\/.*$/, "");
    const selectRepo = (slug) => {
      elForm.action = `${root}/repos/${slug}`;
    };
    const repoOption = (repo) => {
      if (!repo || !repo.slug) return null;
      let option = elRepo.querySelector(`option[value="${CSS.escape(repo.slug)}"]:not([data-view-all-repos])`);
      if (!option && !isAdvanced && repo.name) {
        const viewAll = elRepo.querySelector("[data-view-all-repos]");
        option = new Option(repo.name, repo.slug);
        viewAll.before(option);
      }
      return option;
    };

    // Every search form uses the same last submitted repository.
    const last = repoOption(savedRepo);
    if (last) elRepo.value = last.value;
    if (elRepo.value) selectRepo(elRepo.value);
    elRepo.addEventListener("change", (e) => {
      if (elRepo.selectedOptions[0].hasAttribute("data-view-all-repos")) {
        window.location.assign(elRepo.value);
        return;
      }
      selectRepo(e.target.value);
    });

    // Keep search URLs clean by excluding empty search params.
    elForm.addEventListener("submit", () => {
      if (elRepo.value && !elRepo.selectedOptions[0].hasAttribute("data-view-all-repos")) {
        localStorage.setItem("repo", JSON.stringify({
          name: elRepo.selectedOptions[0].textContent,
          slug: elRepo.value,
        }));
      }
      const empty = Array.from(elForm.querySelectorAll("input, select")).filter((el) => !el.value.trim());
      empty.forEach((el) => (el.disabled = true));
      setTimeout(() => empty.forEach((el) => (el.disabled = false)), 0);
    });

    // On / press, focus the search input.
    document.addEventListener("keydown", (e) => {
      if (elForm !== document.querySelector("[data-search-form]") || e.key !== "/" || e.target.matches("input, select, textarea")) {
        return;
      }

      e.preventDefault();
      elQ.focus();
      elQ.select();
    });
  });

  // Attach autocomplete via /api/suggest/{field}.
  document.querySelectorAll("[data-autocomp]").forEach((el) => {
    autocomp(el, {
      onQuery: async (val) => {
        try {
          const resp = await fetch(`/api/suggest/${el.dataset.autocomp}?q=${encodeURIComponent(val)}`);
          const { data } = await resp.json();
          return data || [];
        } catch {
          return [];
        }
      },
      onSelect: (val) => val,
    });
  });
})();
