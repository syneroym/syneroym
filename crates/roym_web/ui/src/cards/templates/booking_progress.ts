export interface BookingProgressData {
  progress?: string | number;
}

export function renderBookingProgress(data?: BookingProgressData): HTMLElement {
  const div = document.createElement("div");
  div.className = "card card-booking-progress";
  const title = document.createElement("h3");
  title.textContent = "Booking Progress";
  div.appendChild(title);
  const p = document.createElement("p");
  p.textContent =
    data?.progress !== undefined && data?.progress !== null && data?.progress !== ""
      ? `Progress: ${data.progress}`
      : "Booking Progress v1";
  div.appendChild(p);
  return div;
}
