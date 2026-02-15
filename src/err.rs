#[derive(Debug)]
pub enum Error {
	MissingUser,

	UnAuthorised,
	ExpiredAuth,
}
