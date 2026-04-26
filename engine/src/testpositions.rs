#[allow(dead_code)]
use core::str::FromStr;
use super::*;
use position::Position;
use board::Board;

//TODO: finish tests for rest of pieces,Draw and stalemate detection

#[test]
fn test_default(){
    let fen = "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - - 1";
    let board: Board = 
        Board::from_str(fen).unwrap();
    assert_eq!(Board::default(), board);
    assert_eq!(board.get_turn_color(), Color::Red);
}
#[test]
fn test_check(){
    let fen = "3rkr3/9/9/9/9/9/9/9/3KA4/9 w - - - 1";
    let board: Board = 
        Board::from_str(fen).unwrap();
    assert_eq!(board.get_turn_color(), Color::Red);
    assert!(board.is_in_check(RED));
}
#[test]
fn test_mate(){
    let fen = "1R7/4kc3/9/9/9/9/9/9/4A4/3KC4 w - - - 1";
    let board: Board = 
        Board::from_str(fen).unwrap();
    assert!(board.is_checkmate());
}
#[test]
fn test_cannon(){
    let fen = "4k4/4a4/9/4n4/9/9/4C4/9/9/4K4 w - - - 1";
    let board: Board = 
        Board::from_str(fen).unwrap();
    assert!(board.is_legal_move(Move::Piece(Position::new(3, 4), Position::new(5, 4)), RED));
    assert!(!board.is_legal_move(Move::Piece(Position::new(3, 4), Position::new(6, 4)), RED));
    assert!(board.is_threatened(Position::new(8, 4),BLACK));

}
#[test]
fn test_pawn(){
    let fen = "4k4/4a4/9/4n4/2P6/6P2/4C4/9/9/4K4 w - - - 1";
    let board: Board = 
        Board::from_str(fen).unwrap();
    assert!(board.is_legal_move(Move::Piece(Position::new(4, 6), Position::new(5, 6)), RED));
    assert!(!board.is_legal_move(Move::Piece(Position::new(4, 6), Position::new(7, 6)), RED));
    assert!(!board.is_legal_move(Move::Piece(Position::new(5, 2), Position::new(7, 2)), RED));
    assert!(board.is_legal_move(Move::Piece(Position::new(5, 2), Position::new(5, 3)), RED));
}
#[test]
fn test_horse(){
    let fen = "4k4/9/9/5c3/4pn3/9/9/9/9/3K5 w - - - 1";
    let board: Board = 
        Board::from_str(fen).unwrap();
    assert!(board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(6, 7)), BLACK));
    assert!(board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(4, 7)), BLACK));
    assert!(board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(3, 4)), BLACK));
    assert!(board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(3, 6)), BLACK));

    assert!(!board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(7, 4)), BLACK));
    assert!(!board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(7, 6)), BLACK));
    assert!(!board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(4, 3)), BLACK));
    assert!(!board.is_legal_move(Move::Piece(Position::new(5, 5), Position::new(6, 3)), BLACK));
}
/*#[test]
fn test_elephant(){
    let fen = "1R7/4kc3/9/9/9/9/9/9/4A4/3KC4 w - - - 1";
    let board: Board = 
        Board::from_str(fen).unwrap();
} */