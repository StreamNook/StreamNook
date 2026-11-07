// How the drops bar and drawer draw a reward's status: the time strong, the
// rest quiet. The words themselves come from Rust (services/watch_drops.rs).

/** "12m to watch" splits into "12m" and " to watch"; a status with no leading
 *  time ("Ready", "Starts when you watch") is all `rest`. */
export function splitStatus(status: string): { time: string; rest: string } {
  const m = /^((?:\d+d )?(?:\d+h )?\d+[dhm])(\s.*)?$/.exec(status);
  return m ? { time: m[1], rest: m[2] ?? '' } : { time: '', rest: status };
}
