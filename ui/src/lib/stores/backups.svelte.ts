/** Names of the SQL backups open in panes (`backup:<id>` leaves), for their titles. */
class Backups {
  names = $state<Record<string, string>>({});
}

export const backups = new Backups();
