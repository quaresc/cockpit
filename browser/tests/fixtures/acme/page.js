document.getElementById("search").addEventListener("submit", (event) => {
  event.preventDefault();
  document.getElementById("result").textContent = `1 order for ${document.getElementById("q").value}`;
});
